/**
 * Functional utilities — debounce, throttle, memoize, groupBy, deepClone.
 */

export function debounce(fn, wait) {
  let timer;
  return function (...args) {
    clearTimeout(timer);
    timer = setTimeout(() => fn.apply(this, args), wait);
  };
}

export function throttle(fn, limit) {
  let inThrottle = false;
  return function (...args) {
    if (inThrottle) return;
    fn.apply(this, args);
    inThrottle = true;
    setTimeout(() => { inThrottle = false; }, limit);
  };
}

export function memoize(fn) {
  const cache = new Map();
  return function (...args) {
    const key = JSON.stringify(args);
    if (cache.has(key)) return cache.get(key);
    const result = fn.apply(this, args);
    cache.set(key, result);
    return result;
  };
}

export function groupBy(arr, keyFn) {
  return arr.reduce((acc, item) => {
    const key = keyFn(item);
    (acc[key] ??= []).push(item);
    return acc;
  }, Object.create(null));
}

export function deepClone(obj) {
  if (obj === null || typeof obj !== "object") return obj;
  if (obj instanceof Date)  return new Date(obj.getTime());
  if (obj instanceof Array) return obj.map(deepClone);
  return Object.fromEntries(
    Object.entries(obj).map(([k, v]) => [k, deepClone(v)])
  );
}

export function pipe(...fns) {
  return (x) => fns.reduce((v, f) => f(v), x);
}
