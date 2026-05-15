/**
 * Lightweight fetch wrapper with retry, timeout, and JSON handling.
 */

class ApiError extends Error {
  constructor(status, message, body) {
    super(message);
    this.status = status;
    this.body   = body;
  }
}

async function withTimeout(promise, ms) {
  const timeout = new Promise((_, reject) =>
    setTimeout(() => reject(new Error(`Request timed out after ${ms}ms`)), ms)
  );
  return Promise.race([promise, timeout]);
}

async function request(url, options = {}, retries = 2, timeoutMs = 8000) {
  for (let attempt = 0; attempt <= retries; attempt++) {
    try {
      const res = await withTimeout(fetch(url, options), timeoutMs);
      if (!res.ok) {
        const body = await res.text().catch(() => "");
        throw new ApiError(res.status, `HTTP ${res.status}`, body);
      }
      const contentType = res.headers.get("content-type") ?? "";
      return contentType.includes("application/json") ? res.json() : res.text();
    } catch (err) {
      if (attempt === retries || err instanceof ApiError) throw err;
      await new Promise(r => setTimeout(r, 200 * 2 ** attempt));
    }
  }
}

export const api = {
  get:    (url, opts)        => request(url, { ...opts, method: "GET" }),
  post:   (url, body, opts)  => request(url, { ...opts, method: "POST",  body: JSON.stringify(body), headers: { "Content-Type": "application/json", ...opts?.headers } }),
  put:    (url, body, opts)  => request(url, { ...opts, method: "PUT",   body: JSON.stringify(body), headers: { "Content-Type": "application/json", ...opts?.headers } }),
  delete: (url, opts)        => request(url, { ...opts, method: "DELETE" }),
};
