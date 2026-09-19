/**
 * Mother Claude — Claude HTTP bridge client.
 *
 * Works unchanged in a browser and in Node 18+. Every helper takes the same
 * connection options, so a single object configures the whole client:
 *
 *   const conn = { baseUrl: "http://127.0.0.1:5612", token: "…" };
 *
 * An omitted, null or blank `baseUrl` uses the current page's origin in a
 * browser, and `http://127.0.0.1:5612` in Node. An explicit URL keeps whatever
 * path prefix it carries, so the same client works against the dedicated bridge
 * port and against the `/v1` mount on the main app port.
 *
 * `token` may be omitted when the listener runs tokenless.
 *
 * @example
 *   import { chat } from "./client.mjs";
 *   const reply = await chat({ message: "Summarize README.md", cwd: "/repo" });
 *   console.log(reply.response);
 */

const DEFAULT_NODE_BASE = 'http://127.0.0.1:5612';

/** A non-2xx response, carrying the bridge's error envelope. */
export class BridgeApiError extends Error {
  constructor(status, body) {
    const error = (body && body.error) || {};
    super(error.message || `Bridge request failed with HTTP ${status}`);
    this.name = 'BridgeApiError';
    this.status = status;
    /** Stable machine-readable code, e.g. "busy", "thread_not_found". */
    this.code = error.code || 'unknown';
    /** The whole envelope, including context fields like thread_id or field. */
    this.details = error;
    this.body = body;
  }
}

function resolveBaseUrl(baseUrl) {
  if (typeof baseUrl === 'string' && baseUrl.trim()) {
    return baseUrl.trim().replace(/\/+$/, '');
  }
  if (typeof globalThis.location === 'object' && globalThis.location && globalThis.location.origin) {
    const { origin, pathname } = globalThis.location;
    // Serving the example page from /example/ means the API sits one level up.
    const prefix = pathname.replace(/\/(example|docs)\/?.*$/, '');
    return (origin + prefix).replace(/\/+$/, '');
  }
  return DEFAULT_NODE_BASE;
}

function authHeaders(token) {
  return typeof token === 'string' && token.trim()
    ? { authorization: `Bearer ${token.trim()}` }
    : {};
}

async function parse(response) {
  const text = await response.text();
  let body = null;
  if (text) {
    try {
      body = JSON.parse(text);
    } catch {
      body = { error: { code: 'invalid_json', message: text.slice(0, 500) } };
    }
  }
  if (!response.ok) throw new BridgeApiError(response.status, body);
  return body;
}

async function request(conn, method, path, { body, query, signal, raw } = {}) {
  const base = resolveBaseUrl(conn && conn.baseUrl);
  const url = new URL(base + path, base + '/');
  for (const [key, value] of Object.entries(query || {})) {
    if (value !== undefined && value !== null && value !== '') url.searchParams.set(key, value);
  }
  const init = {
    method,
    signal,
    headers: { ...authHeaders(conn && conn.token) },
  };
  if (body instanceof FormData) {
    // Never set Content-Type for FormData; fetch adds the boundary.
    init.body = body;
  } else if (body !== undefined) {
    init.headers['content-type'] = 'application/json';
    init.body = JSON.stringify(body);
  }
  const response = await fetch(url, init);
  if (raw) {
    if (!response.ok) await parse(response);
    return response;
  }
  return parse(response);
}

// --- discovery -------------------------------------------------------------

export const getHealth = (conn = {}) => request(conn, 'GET', '/health');
export const getCapabilities = (conn = {}) => request(conn, 'GET', '/capabilities');
export const getMetadata = (conn = {}) => request(conn, 'GET', '/metadata');
export const listModels = (conn = {}) => request(conn, 'GET', '/models');

// --- conversations ---------------------------------------------------------

/** Start a conversation. Returns `{ thread_id, status, cwd, … }`. */
export const createThread = (conn = {}, options = {}) =>
  request(conn, 'POST', '/threads', { body: options });

export const listThreads = (conn = {}, query = {}) =>
  request(conn, 'GET', '/threads', { query });

export const readThread = (conn = {}, threadId, query = {}) =>
  request(conn, 'GET', `/threads/${encodeURIComponent(threadId)}`, { query });

export const threadMessages = (conn = {}, threadId, query = {}) =>
  request(conn, 'GET', `/threads/${encodeURIComponent(threadId)}/messages`, { query });

export const threadContext = (conn = {}, threadId) =>
  request(conn, 'GET', `/threads/${encodeURIComponent(threadId)}/context`);

export const renameThread = (conn = {}, threadId, name) =>
  request(conn, 'POST', `/threads/${encodeURIComponent(threadId)}/name`, { body: { name } });

export const forkThread = (conn = {}, threadId, options = {}) =>
  request(conn, 'POST', `/threads/${encodeURIComponent(threadId)}/fork`, { body: options });

export const setThreadModel = (conn = {}, threadId, model) =>
  request(conn, 'POST', `/threads/${encodeURIComponent(threadId)}/model`, { body: { model } });

export const setPermissionMode = (conn = {}, threadId, permissionMode) =>
  request(conn, 'POST', `/threads/${encodeURIComponent(threadId)}/permission-mode`, {
    body: { permission_mode: permissionMode },
  });

export const closeThread = (conn = {}, threadId, { purge = false } = {}) =>
  request(conn, 'DELETE', `/threads/${encodeURIComponent(threadId)}`, {
    query: purge ? { purge: 'true' } : {},
  });

// --- turns -----------------------------------------------------------------

// Transport-only options must never reach the request body: the server rejects
// unknown fields, so a caller passing an AbortSignal would get a 422 rather
// than a cancellable turn.
const splitOptions = ({ signal, onEvent, after, ...body } = {}) => ({ signal, body });

/** Start a turn. Returns an operation snapshot immediately (HTTP 202). */
export const startTurn = (conn = {}, threadId, input, options = {}) => {
  const { signal, body } = splitOptions(options);
  return request(conn, 'POST', `/threads/${encodeURIComponent(threadId)}/turns`, {
    body: { input, ...body },
    signal,
  });
};

/** Start a turn and wait for it. */
export const runTurn = (conn = {}, threadId, input, options = {}) => {
  const { signal, body } = splitOptions(options);
  return request(conn, 'POST', `/threads/${encodeURIComponent(threadId)}/run`, {
    body: { input, ...body },
    signal,
  });
};

export const steerTurn = (conn = {}, threadId, turnId, input, options = {}) => {
  const { signal, body } = splitOptions(options);
  return request(
    conn,
    'POST',
    `/threads/${encodeURIComponent(threadId)}/turns/${encodeURIComponent(turnId)}/steer`,
    { body: { input, ...body }, signal },
  );
};

export const interruptTurn = (conn = {}, threadId, turnId) =>
  request(conn, 'POST', `/threads/${encodeURIComponent(threadId)}/turns/${encodeURIComponent(turnId)}/interrupt`, {
    body: {},
  });

/**
 * One message in, one answer out. Omit `threadId` to start a conversation;
 * any conversation option (cwd, model, effort, permission_mode, …) may be
 * passed alongside and applies to a newly created one.
 */
export const chat = (conn = {}, { message, input, threadId, ...options } = {}) => {
  const { signal, body } = splitOptions(options);
  return request(conn, 'POST', '/chat', {
    body: {
      ...(message !== undefined ? { message } : {}),
      ...(input !== undefined ? { input } : {}),
      ...(threadId ? { thread_id: threadId } : {}),
      ...body,
    },
    signal,
  });
};

export const getOperation = (conn = {}, operationId) =>
  request(conn, 'GET', `/operations/${encodeURIComponent(operationId)}`);

// --- streaming -------------------------------------------------------------

/**
 * Async-iterate an SSE endpoint. Yields `{ id, data }` where `data` is the
 * parsed JSON payload.
 *
 * Uses fetch rather than EventSource so the token can travel in a header.
 * (EventSource also works against this bridge — pass `?token=` — but cannot set
 * headers, so it leaks the token into server logs and history.)
 */
async function* streamSse(conn, path, { after, signal } = {}) {
  const response = await request(conn, 'GET', path, {
    query: after !== undefined ? { after } : {},
    signal,
    raw: true,
  });
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';

  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });

      let split;
      while ((split = buffer.indexOf('\n\n')) !== -1) {
        const frame = buffer.slice(0, split);
        buffer = buffer.slice(split + 2);

        let id = null;
        const data = [];
        for (const line of frame.split('\n')) {
          if (line.startsWith('id:')) id = line.slice(3).trim();
          else if (line.startsWith('data:')) data.push(line.slice(5).replace(/^ /, ''));
          // Lines starting with ':' are heartbeat comments.
        }
        if (!data.length) continue;
        const text = data.join('\n');
        let parsed;
        try {
          parsed = JSON.parse(text);
        } catch {
          parsed = { raw: text };
        }
        yield { id, data: parsed };
      }
    }
  } finally {
    reader.cancel().catch(() => {});
  }
}

/** Stream one turn's events. Resume with `{ after: lastEventId }`. */
export const streamOperation = (conn = {}, operationId, options = {}) =>
  streamSse(conn, `/operations/${encodeURIComponent(operationId)}/events`, options);

/** Stream every event across every conversation. */
export const streamEvents = (conn = {}, options = {}) => streamSse(conn, '/events', options);

/**
 * Run a turn and stream it, returning the terminal result.
 *
 * `onEvent` receives every frame; `bridge/completed` and `bridge/error` end the
 * stream. This is the shape most applications want: start, watch, settle.
 */
export async function runTurnStreaming(conn = {}, threadId, input, { onEvent, ...options } = {}) {
  const operation = await startTurn(conn, threadId, input, options);
  let terminal = null;
  for await (const { data } of streamOperation(conn, operation.operation_id, options)) {
    if (onEvent) onEvent(data);
    if (data.method === 'bridge/completed' || data.method === 'bridge/error') {
      terminal = data.params;
      break;
    }
  }
  return terminal || (await getOperation(conn, operation.operation_id));
}

// --- tool approvals and questions -----------------------------------------

export const listRequests = (conn = {}) => request(conn, 'GET', '/requests');

/**
 * Answer a blocked request. Pass exactly one of:
 *   { decision: "allow" | "deny", message?, updated_input?, updated_permissions? }
 *   { answer: "…" }            for a question
 *   { result: { … } }          to send the raw callback result
 */
export const respondToRequest = (conn = {}, requestId, body) =>
  request(conn, 'POST', `/requests/${encodeURIComponent(requestId)}/respond`, { body });

// --- direct Messages API ---------------------------------------------------

/** One `POST /v1/messages` call. Needs ANTHROPIC_API_KEY on the server. */
export const createMessage = (conn = {}, body) => request(conn, 'POST', '/messages', { body });

/** Stream a Messages API call; yields raw upstream SSE frames. */
export async function* streamMessage(conn = {}, body, { signal } = {}) {
  const response = await request(conn, 'POST', '/messages', {
    body: { ...body, stream: true },
    signal,
    raw: true,
  });
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      let split;
      while ((split = buffer.indexOf('\n\n')) !== -1) {
        const frame = buffer.slice(0, split);
        buffer = buffer.slice(split + 2);
        const data = frame
          .split('\n')
          .filter((l) => l.startsWith('data:'))
          .map((l) => l.slice(5).trim())
          .join('\n');
        if (!data || data === '[DONE]') continue;
        try {
          yield JSON.parse(data);
        } catch {
          // A partial frame is not worth throwing over.
        }
      }
    }
  } finally {
    reader.cancel().catch(() => {});
  }
}

/** Upload a file to the Files API. `file` is a browser File/Blob or a Buffer. */
export function uploadFile(conn = {}, file, { filename, expiresInSeconds } = {}) {
  const form = new FormData();
  if (filename) {
    form.append('file', file, filename);
    form.append('filename', filename);
  } else {
    form.append('file', file);
  }
  if (expiresInSeconds) form.append('expires_in_seconds', String(expiresInSeconds));
  return request(conn, 'POST', '/files', { body: form });
}

export const listFiles = (conn = {}, query = {}) => request(conn, 'GET', '/files', { query });

export const deleteFile = (conn = {}, fileId) =>
  request(conn, 'DELETE', `/files/${encodeURIComponent(fileId)}`);

// --- input helpers ---------------------------------------------------------

/** Identify an image from its magic bytes, the way the server does. */
function sniffImage(bytes) {
  const starts = (...sig) => sig.every((b, i) => bytes[i] === b);
  if (starts(0x89, 0x50, 0x4e, 0x47)) return 'image/png';
  if (starts(0xff, 0xd8, 0xff)) return 'image/jpeg';
  if (starts(0x47, 0x49, 0x46, 0x38)) return 'image/gif';
  if (starts(0x52, 0x49, 0x46, 0x46) && bytes[8] === 0x57 && bytes[9] === 0x45) return 'image/webp';
  return null;
}

/** Read a browser File/Blob into an image input item. */
export async function imageFromFile(file) {
  const buffer = await file.arrayBuffer();
  const bytes = new Uint8Array(buffer);

  // Trust the bytes over the declared type. A Blob built by hand often has an
  // empty `type`, and claiming image/png for a JPEG is rejected by the server's
  // byte-sniff check — a confusing failure for something it can just work out.
  const mediaType = sniffImage(bytes) ?? file.type;
  if (!mediaType) {
    throw new TypeError('Unrecognised image: expected PNG, JPEG, GIF or WebP.');
  }

  let binary = '';
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  }
  return {
    type: 'image',
    source: { type: 'base64', media_type: mediaType, data: btoa(binary) },
  };
}

/** Text plus any number of images, as a turn input array. */
export const withImages = (text, images = []) => [
  ...images,
  ...(text ? [{ type: 'text', text }] : []),
];
