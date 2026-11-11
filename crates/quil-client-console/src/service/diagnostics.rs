use std::{
    net::{IpAddr, SocketAddr},
    str::FromStr,
    time::{Duration, Instant},
};

use futures::StreamExt;
use libp2p::{
    core::{muxing::StreamMuxerBox, transport::Boxed, upgrade::Version},
    identify,
    identity::Keypair,
    multiaddr::Protocol,
    ping,
    swarm::{NetworkBehaviour, SwarmEvent},
    Multiaddr, PeerId, Swarm, Transport,
};
use quil_p2p::pqnoise_transport::PqNoiseConfig;

use super::ServiceError;
use crate::domain::{
    DiagnosticsReport, PeerAddressReport, PeerAddressSnapshot, PeerReach, PublishedAddressSnapshot,
};

const PROTOCOL_VERSION: &str = "/quilibrium/2.0.2/0";
const TIMEOUT: Duration = Duration::from_secs(10);
const MASTER_FILTER: [u8; 32] = [0xff; 32];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Carrier {
    Tcp,
    Quic,
}

impl Carrier {
    fn label(self) -> &'static str {
        match self {
            Self::Tcp => "TCP · PQNoise · yamux",
            Self::Quic => "QUIC v1 · TLS 1.3 · sntrup761",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Any,
    V4,
    V6,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Host {
    Ip(IpAddr),
    Dns { name: String, family: Family },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    host: Host,
    port: u16,
    carrier: Carrier,
    expected_peer: Option<PeerId>,
}

fn parse_target(value: &str) -> Result<Target, ServiceError> {
    let value = value.trim();
    let address = Multiaddr::from_str(value).map_err(|error| {
        ServiceError::InvalidInput(format!(
            "Invalid Multiaddr: {error}. Example: /ip4/127.0.0.1/tcp/8336"
        ))
    })?;
    let mut parts = address.iter();
    let host = match parts.next() {
        Some(Protocol::Ip4(ip)) => Host::Ip(ip.into()),
        Some(Protocol::Ip6(ip)) => Host::Ip(ip.into()),
        Some(Protocol::Dns(name)) => Host::Dns {
            name: name.to_string(),
            family: Family::Any,
        },
        Some(Protocol::Dns4(name)) => Host::Dns {
            name: name.to_string(),
            family: Family::V4,
        },
        Some(Protocol::Dns6(name)) => Host::Dns {
            name: name.to_string(),
            family: Family::V6,
        },
        _ => {
            return Err(ServiceError::InvalidInput(
                "Address must start with /ip4, /ip6, /dns, /dns4, or /dns6".into(),
            ));
        }
    };
    let (carrier, port) = match parts.next() {
        Some(Protocol::Tcp(port)) => (Carrier::Tcp, port),
        Some(Protocol::Udp(port)) if matches!(parts.next(), Some(Protocol::QuicV1)) => {
            (Carrier::Quic, port)
        }
        _ => {
            return Err(ServiceError::InvalidInput(
                "Address requires /tcp/<port> or /udp/<port>/quic-v1".into(),
            ));
        }
    };
    if port == 0 {
        return Err(ServiceError::InvalidInput("Port cannot be 0".into()));
    }
    let expected_peer = match parts.next() {
        None => None,
        Some(Protocol::P2p(peer)) => Some(peer),
        Some(protocol) => {
            return Err(ServiceError::InvalidInput(format!(
                "Address protocol {protocol} is not supported"
            )));
        }
    };
    if parts.next().is_some() {
        return Err(ServiceError::InvalidInput(
            "No address segments may follow /p2p/<peer-id>".into(),
        ));
    }
    Ok(Target {
        host,
        port,
        carrier,
        expected_peer,
    })
}

async fn resolve(target: &Target) -> Result<SocketAddr, ServiceError> {
    match &target.host {
        Host::Ip(ip) => Ok(SocketAddr::new(*ip, target.port)),
        Host::Dns { name, family } => tokio::net::lookup_host((name.as_str(), target.port))
            .await
            .map_err(|error| ServiceError::Diagnostics(format!("Cannot resolve {name}: {error}")))?
            .find(|address| match family {
                Family::Any => true,
                Family::V4 => address.is_ipv4(),
                Family::V6 => address.is_ipv6(),
            })
            .ok_or_else(|| {
                ServiceError::Diagnostics(format!(
                    "{name} has no record matching the address family"
                ))
            }),
    }
}

fn dial_address(socket: SocketAddr, carrier: Carrier) -> Multiaddr {
    let mut address = Multiaddr::empty();
    address.push(match socket.ip() {
        IpAddr::V4(ip) => Protocol::Ip4(ip),
        IpAddr::V6(ip) => Protocol::Ip6(ip),
    });
    match carrier {
        Carrier::Tcp => address.push(Protocol::Tcp(socket.port())),
        Carrier::Quic => {
            address.push(Protocol::Udp(socket.port()));
            address.push(Protocol::QuicV1);
        }
    }
    address
}

fn tcp_transport(keypair: &Keypair) -> Result<Boxed<(PeerId, StreamMuxerBox)>, ServiceError> {
    let security = PqNoiseConfig::new(keypair)
        .map_err(|error| ServiceError::Diagnostics(error.to_string()))?;
    Ok(
        libp2p::tcp::tokio::Transport::new(libp2p::tcp::Config::default())
            .upgrade(Version::V1Lazy)
            .authenticate(security)
            .multiplex(libp2p::yamux::Config::default())
            .boxed(),
    )
}

fn quic_transport(keypair: &Keypair) -> Boxed<(PeerId, StreamMuxerBox)> {
    libp2p::quic::tokio::Transport::new(libp2p::quic::Config::new(keypair))
        .map(|(peer, connection), _| (peer, StreamMuxerBox::new(connection)))
        .boxed()
}

#[derive(NetworkBehaviour)]
struct DialBehaviour {
    identify: identify::Behaviour,
    ping: ping::Behaviour,
}

pub(super) async fn dial(value: &str) -> Result<DiagnosticsReport, ServiceError> {
    let target = parse_target(value)?;
    let socket = resolve(&target).await?;
    let keypair = Keypair::generate_ed25519();
    let local_peer = keypair.public().to_peer_id();
    let transport = match target.carrier {
        Carrier::Tcp => tcp_transport(&keypair)?,
        Carrier::Quic => quic_transport(&keypair),
    };
    let behaviour = DialBehaviour {
        identify: identify::Behaviour::new(
            identify::Config::new(PROTOCOL_VERSION.into(), keypair.public())
                .with_agent_version(format!("qclient-console/{}", env!("CARGO_PKG_VERSION")))
                .with_push_listen_addr_updates(false)
                .with_cache_size(0),
        ),
        ping: ping::Behaviour::new(ping::Config::new().with_timeout(TIMEOUT)),
    };
    let config =
        libp2p::swarm::Config::with_tokio_executor().with_idle_connection_timeout(TIMEOUT * 2);
    let mut swarm = Swarm::new(transport, behaviour, local_peer, config);
    let dialed = dial_address(socket, target.carrier);
    swarm
        .dial(dialed.clone())
        .map_err(|error| ServiceError::Diagnostics(format!("Cannot dial {dialed}: {error}")))?;

    let started = Instant::now();
    let mut deadline = started + TIMEOUT;
    let mut peer = None;
    let mut connection_ms = 0;
    let mut ping_ms = None;
    let mut ping_done = false;
    let mut identify_done = false;
    let mut agent_version = None;
    let mut protocol_version = None;
    let mut observed_address = None;
    let mut listen_addresses = Vec::new();
    let mut protocols = Vec::new();
    while peer.is_none() || !ping_done || !identify_done {
        let Ok(event) = tokio::time::timeout_at(deadline.into(), swarm.select_next_some()).await
        else {
            break;
        };
        match event {
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                if target
                    .expected_peer
                    .is_some_and(|expected| expected != peer_id)
                {
                    return Err(ServiceError::Diagnostics(format!(
                        "Peer ID mismatch: expected {}, got {peer_id}",
                        target.expected_peer.expect("checked above")
                    )));
                }
                connection_ms = started.elapsed().as_millis();
                peer = Some(peer_id);
                deadline = Instant::now() + TIMEOUT;
            }
            SwarmEvent::OutgoingConnectionError { error, .. } => {
                return Err(ServiceError::Diagnostics(format!(
                    "Connection to {dialed} failed: {error}"
                )));
            }
            SwarmEvent::Behaviour(DialBehaviourEvent::Identify(identify::Event::Received {
                info,
                ..
            })) => {
                agent_version = Some(info.agent_version);
                protocol_version = Some(info.protocol_version);
                observed_address = Some(printable(&info.observed_addr.to_string()));
                listen_addresses = info
                    .listen_addrs
                    .iter()
                    .map(|address| {
                        let multiaddr = printable(&address.to_string());
                        PublishedAddressSnapshot {
                            reach: multiaddr_reach(&multiaddr),
                            multiaddr,
                        }
                    })
                    .collect();
                protocols = info.protocols.iter().map(ToString::to_string).collect();
                protocols.sort();
                protocols.dedup();
                identify_done = true;
            }
            SwarmEvent::Behaviour(DialBehaviourEvent::Ping(ping::Event {
                result: Ok(duration),
                ..
            })) => {
                ping_ms = Some(duration.as_millis());
                ping_done = true;
            }
            SwarmEvent::Behaviour(DialBehaviourEvent::Ping(ping::Event {
                result: Err(_), ..
            })) => ping_done = true,
            SwarmEvent::Behaviour(DialBehaviourEvent::Identify(identify::Event::Error {
                ..
            })) => identify_done = true,
            SwarmEvent::ConnectionClosed { cause, .. } if peer.is_some() => {
                if ping_ms.is_none() && agent_version.is_none() {
                    return Err(ServiceError::Diagnostics(format!(
                        "Connected, but the peer disconnected early: {}",
                        cause
                            .map(|error| error.to_string())
                            .unwrap_or_else(|| "no error details".into())
                    )));
                }
                break;
            }
            _ => {}
        }
    }

    let peer = peer.ok_or_else(|| {
        ServiceError::Diagnostics(format!(
            "Connection to {dialed} exceeded {} seconds",
            TIMEOUT.as_secs()
        ))
    })?;
    Ok(DiagnosticsReport {
        address: value.trim().into(),
        peer_id: peer.to_string(),
        transport: target.carrier.label().into(),
        connection_ms,
        ping_ms,
        agent_version,
        protocol_version,
        observed_address,
        listen_addresses,
        protocols,
    })
}

pub(super) async fn resolve_peer(
    config_dir: &std::path::Path,
    value: &str,
) -> Result<PeerAddressReport, ServiceError> {
    let target = PeerId::from_str(value.trim())
        .map_err(|error| ServiceError::InvalidInput(format!("Invalid Peer ID: {error}")))?;
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let address = super::node::grpc_address(config.effective_listen_grpc_multiaddr())?;
    let mut client = super::node::connect_client(&address).await?;
    let response = tokio::time::timeout(
        super::node::RPC_TIMEOUT,
        client.get_peer_info(tonic::Request::new(
            quil_types::proto::node::GetPeerInfoRequest {},
        )),
    )
    .await
    .map_err(|_| ServiceError::Timeout)?
    .map_err(|error| ServiceError::Rpc(error.to_string()))?
    .into_inner();

    map_peer_addresses(&response.peer_info, &target)
}

fn map_peer_addresses(
    peers: &[quil_types::proto::node::PeerInfo],
    target: &PeerId,
) -> Result<PeerAddressReport, ServiceError> {
    let peer = peers
        .iter()
        .find(|peer| peer.peer_id == target.to_bytes())
        .ok_or_else(|| {
            ServiceError::Diagnostics(format!(
                "This node has no Peer information for {target}; it can resolve only peers whose announcements it has received"
            ))
        })?;
    let mut addresses = Vec::new();
    for reachability in &peer.reachability {
        let filter = match reachability.filter.as_slice() {
            value if value == MASTER_FILTER => "master".into(),
            [] => "-".into(),
            value => hex::encode(value),
        };
        for (kind, values) in [
            ("stream", &reachability.stream_multiaddrs),
            ("pubsub", &reachability.pubsub_multiaddrs),
        ] {
            for value in values.iter().filter(|value| !value.is_empty()) {
                addresses.push(PeerAddressSnapshot {
                    kind: kind.into(),
                    host_port: multiaddr_host_port(value),
                    reach: multiaddr_reach(value),
                    multiaddr: printable(value),
                    filter: filter.clone(),
                });
            }
        }
    }
    if addresses.is_empty() {
        return Err(ServiceError::Diagnostics(format!(
            "{target} has not published any addresses"
        )));
    }
    Ok(PeerAddressReport {
        peer_id: target.to_string(),
        addresses,
    })
}

fn multiaddr_host_port(value: &str) -> Option<String> {
    let address = Multiaddr::from_str(value).ok()?;
    let mut host = None;
    let mut port = None;
    for protocol in address.iter() {
        match protocol {
            Protocol::Ip4(ip) => host = Some(ip.to_string()),
            Protocol::Ip6(ip) => host = Some(format!("[{ip}]")),
            Protocol::Dns(name) | Protocol::Dns4(name) | Protocol::Dns6(name) => {
                host = Some(name.to_string())
            }
            Protocol::Tcp(value) | Protocol::Udp(value) => port = Some(value),
            _ => {}
        }
    }
    host.zip(port).map(|(host, port)| format!("{host}:{port}"))
}

fn multiaddr_reach(value: &str) -> PeerReach {
    let Ok(address) = Multiaddr::from_str(value) else {
        return PeerReach::Unknown;
    };
    for protocol in address.iter() {
        match protocol {
            Protocol::Ip4(ip) => return ip_reach(IpAddr::V4(ip)),
            Protocol::Ip6(ip) => return ip_reach(IpAddr::V6(ip)),
            Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_) => return PeerReach::Unknown,
            _ => {}
        }
    }
    PeerReach::Unknown
}

fn ip_reach(ip: IpAddr) -> PeerReach {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            let shared = octets[0] == 100 && (octets[1] & 0xc0) == 0x40;
            if ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || shared
            {
                PeerReach::Private
            } else {
                PeerReach::Public
            }
        }
        IpAddr::V6(ip) => {
            if let Some(ip) = ip.to_ipv4_mapped() {
                return ip_reach(IpAddr::V4(ip));
            }
            if ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
            {
                PeerReach::Private
            } else {
                PeerReach::Public
            }
        }
    }
}

fn printable(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(200)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quil_types::proto::node::{PeerInfo, Reachability};

    #[test]
    fn parses_tcp_quic_and_optional_peer() {
        let tcp = parse_target("/ip4/127.0.0.1/tcp/8336").unwrap();
        assert_eq!(tcp.carrier, Carrier::Tcp);
        assert_eq!(tcp.port, 8336);

        let quic = parse_target("/dns4/node.example/udp/8336/quic-v1").unwrap();
        assert_eq!(quic.carrier, Carrier::Quic);
        assert_eq!(
            quic.host,
            Host::Dns {
                name: "node.example".into(),
                family: Family::V4,
            }
        );
    }

    #[test]
    fn rejects_unsupported_and_incomplete_addresses() {
        assert!(parse_target("127.0.0.1:8336").is_err());
        assert!(parse_target("/ip4/127.0.0.1/udp/8336").is_err());
        assert!(parse_target("/ip4/127.0.0.1/tcp/0").is_err());
    }

    #[test]
    fn maps_only_the_requested_peer_and_classifies_published_addresses() {
        let target = quil_p2p::PeerId::random();
        let other = quil_p2p::PeerId::random();
        let peers = vec![
            PeerInfo {
                peer_id: other.to_bytes(),
                reachability: vec![Reachability {
                    filter: vec![0xff; 32],
                    stream_multiaddrs: vec!["/ip4/203.0.113.1/tcp/8340".into()],
                    ..Default::default()
                }],
                ..Default::default()
            },
            PeerInfo {
                peer_id: target.to_bytes(),
                reachability: vec![
                    Reachability {
                        filter: vec![0xff; 32],
                        stream_multiaddrs: vec!["/ip4/192.168.1.20/tcp/8340".into()],
                        pubsub_multiaddrs: vec!["/dns4/node.example/udp/8336/quic-v1".into()],
                    },
                    Reachability {
                        filter: vec![0xaa; 32],
                        stream_multiaddrs: vec!["/ip4/203.0.113.8/tcp/8340".into()],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        ];

        let report = map_peer_addresses(&peers, &target).unwrap();

        assert_eq!(report.peer_id, target.to_string());
        assert_eq!(report.addresses.len(), 3);
        assert_eq!(report.addresses[0].filter, "master");
        assert_eq!(report.addresses[0].reach, crate::domain::PeerReach::Private);
        assert_eq!(
            report.addresses[0].host_port.as_deref(),
            Some("192.168.1.20:8340")
        );
        assert_eq!(report.addresses[1].kind, "pubsub");
        assert_eq!(report.addresses[1].reach, crate::domain::PeerReach::Unknown);
        assert_eq!(report.addresses[2].filter, "aa".repeat(32));
        assert_eq!(report.addresses[2].reach, crate::domain::PeerReach::Public);
    }
}
