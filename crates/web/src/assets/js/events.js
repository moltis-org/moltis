const eventListeners = {};
function onEvent(eventName, handler) {
  (eventListeners[eventName] = eventListeners[eventName] || []).push(handler);
  return function off() {
    const arr = eventListeners[eventName];
    if (arr) {
      const idx = arr.indexOf(handler);
      if (idx !== -1) arr.splice(idx, 1);
    }
  };
}
export {
  eventListeners,
  onEvent
};
