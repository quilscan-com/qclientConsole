export function createIdleTimer({ timeoutMs, onIdle, setTimer = setTimeout, clearTimer = clearTimeout }) {
  let handle

  function stop() {
    if (handle === undefined) return
    clearTimer(handle)
    handle = undefined
  }

  function start() {
    stop()
    handle = setTimer(() => {
      handle = undefined
      onIdle()
    }, timeoutMs)
  }

  return { start, reset: start, stop }
}
