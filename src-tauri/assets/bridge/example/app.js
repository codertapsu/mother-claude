/**
 * The bridge console. Deliberately plain: one module, no framework, no build.
 * It is meant to be read as a worked example of the client API as much as used.
 */
import {
  BridgeApiError,
  chat,
  createThread,
  getHealth,
  imageFromFile,
  interruptTurn,
  listModels,
  listRequests,
  respondToRequest,
  startTurn,
  streamOperation,
  withImages,
} from '../client.mjs';

const $ = (id) => document.getElementById(id);
const state = { threadId: null, turnId: null, operationId: null, pollTimer: null };

/** Current connection options, read fresh so edits take effect immediately. */
const conn = () => ({ baseUrl: $('base-url').value, token: $('token').value });

function say(el, text, kind = '') {
  el.textContent = text;
  el.className = `status ${kind}`;
}

function describe(err) {
  if (err instanceof BridgeApiError) {
    const field = err.details.field ? ` (${err.details.field})` : '';
    return `${err.code}${field}: ${err.message}`;
  }
  return String(err && err.message ? err.message : err);
}

// --- stream rendering ------------------------------------------------------

function line(kind, text) {
  const el = document.createElement('div');
  el.className = `line ${kind}`;
  el.textContent = text;
  const stream = $('stream');
  stream.append(el);
  stream.scrollTop = stream.scrollHeight;
  return el;
}

/** Render one bridge event. Only the interesting shapes get special treatment. */
function renderEvent(event) {
  const { method, params } = event;

  if (method === 'turn/started') {
    line('meta', `▶ turn ${params.turn_id.slice(0, 8)} — ${params.text || '(no text)'}`);
    return;
  }
  if (method === 'turn/steered') {
    line('meta', `↻ steered: ${params.text}`);
    return;
  }
  if (method === 'bridge/request') {
    refreshRequests();
    return;
  }
  if (method === 'bridge/completed') {
    const result = params.result || {};
    line('done', `✓ ${result.result ?? '(no text)'}`);
    if (result.total_cost_usd) line('meta', `  cost $${result.total_cost_usd.toFixed(4)}`);
    return;
  }
  if (method === 'bridge/error') {
    line('error', `✗ ${params.error ? params.error.message : 'failed'}`);
    return;
  }
  if (method !== 'message') return;

  const message = params.message || {};
  if (message.type === 'assistant') {
    for (const block of message.message?.content || []) {
      if (block.type === 'text') line('assistant', block.text);
      else if (block.type === 'thinking') line('thinking', `💭 ${block.thinking}`);
      else if (block.type === 'tool_use') {
        line('tool', `🔧 ${block.name} ${JSON.stringify(block.input).slice(0, 160)}`);
      }
    }
  } else if (message.type === 'system' && message.subtype === 'init') {
    line('meta', `● ${message.model} · ${message.tools?.length ?? 0} tools · ${message.cwd}`);
  }
}

// --- pending requests ------------------------------------------------------

async function refreshRequests() {
  let pending = [];
  try {
    pending = (await listRequests(conn())).data || [];
  } catch {
    return;
  }
  const panel = $('requests-panel');
  const host = $('requests');
  host.replaceChildren();
  panel.hidden = pending.length === 0;

  for (const request of pending) {
    const card = document.createElement('div');
    card.className = 'request';

    const title = document.createElement('p');
    title.className = 'request-title';
    title.textContent =
      request.params?.prompt ||
      (request.kind === 'question' ? 'Claude has a question' : 'Claude wants to use a tool');
    card.append(title);

    if (request.params?.detail) {
      const detail = document.createElement('pre');
      detail.textContent = request.params.detail;
      card.append(detail);
    }

    const actions = document.createElement('div');
    actions.className = 'row';

    if (request.kind === 'question') {
      const answer = document.createElement('input');
      answer.type = 'text';
      answer.placeholder = 'Your answer';
      const send = document.createElement('button');
      send.type = 'button';
      send.textContent = 'Answer';
      send.onclick = () => answerRequest(request.request_id, { answer: answer.value });
      for (const option of request.params?.options || []) {
        const label = typeof option === 'string' ? option : option.label;
        const pick = document.createElement('button');
        pick.type = 'button';
        pick.className = 'ghost';
        pick.textContent = label;
        pick.onclick = () => answerRequest(request.request_id, { answer: label });
        actions.append(pick);
      }
      actions.append(answer, send);
    } else {
      const allow = document.createElement('button');
      allow.type = 'button';
      allow.textContent = request.dangerous ? 'Allow (dangerous)' : 'Allow';
      if (request.dangerous) allow.className = 'danger';
      allow.onclick = () => answerRequest(request.request_id, { decision: 'allow' });

      const deny = document.createElement('button');
      deny.type = 'button';
      deny.className = 'ghost';
      deny.textContent = 'Deny';
      deny.onclick = () => answerRequest(request.request_id, { decision: 'deny' });

      actions.append(allow, deny);
    }

    card.append(actions);
    host.append(card);
  }
}

async function answerRequest(requestId, body) {
  try {
    await respondToRequest(conn(), requestId, body);
    line('meta', `↩ answered ${requestId.slice(0, 8)}`);
  } catch (err) {
    line('error', describe(err));
  }
  refreshRequests();
}

// --- actions ---------------------------------------------------------------

$('connect').onclick = async () => {
  try {
    const health = await getHealth(conn());
    say(
      $('health'),
      `${health.status} · agent SDK ${health.host_built ? 'ready' : 'not built'} · ` +
        `Messages API ${health.messages_api ? 'on' : 'off'} · v${health.version}`,
      health.status === 'ready' ? 'ok' : 'error',
    );
    const models = await listModels(conn());
    const select = $('model');
    select.replaceChildren(new Option('default', ''));
    for (const model of models.data || []) {
      select.append(new Option(model.displayName || model.value, model.value));
    }
  } catch (err) {
    // /health answers 503 with a *body* when the runtime is not built — that
    // body is the useful part, so read it rather than only the status line.
    const body = err instanceof BridgeApiError ? err.body : null;
    if (body && body.status) {
      say(
        $('health'),
        `${body.status} · agent SDK ${body.host_built ? 'ready' : 'not built — run npm run sidecar:build'}`,
        'error',
      );
    } else {
      say($('health'), describe(err), 'error');
    }
  }
};

$('create').onclick = async () => {
  try {
    const created = await createThread(conn(), {
      ...($('cwd').value ? { cwd: $('cwd').value } : {}),
      ...($('model').value ? { model: $('model').value } : {}),
      permission_mode: $('permission-mode').value,
    });
    state.threadId = created.thread_id;
    $('thread-id').value = created.thread_id;
    line('meta', `● conversation ${created.thread_id} in ${created.cwd}`);
    say($('send-status'), 'Ready.', 'ok');
  } catch (err) {
    say($('send-status'), describe(err), 'error');
  }
};

$('send').onclick = async () => {
  const threadId = $('thread-id').value.trim() || state.threadId;
  const text = $('message').value.trim();
  const files = Array.from($('images').files || []);
  if (!text && !files.length) {
    say($('send-status'), 'Type a message or attach an image.', 'error');
    return;
  }

  try {
    const images = await Promise.all(files.map(imageFromFile));
    const input = images.length ? withImages(text, images) : text;

    // No conversation yet? /chat creates one and answers in a single call.
    if (!threadId) {
      say($('send-status'), 'Starting a conversation…');
      const reply = await chat(conn(), {
        input,
        ...($('cwd').value ? { cwd: $('cwd').value } : {}),
        ...($('model').value ? { model: $('model').value } : {}),
        permission_mode: $('permission-mode').value,
      });
      state.threadId = reply.thread_id;
      $('thread-id').value = reply.thread_id;
      line('done', `✓ ${reply.response ?? '(no text)'}`);
      say($('send-status'), 'Done.', 'ok');
      return;
    }

    say($('send-status'), 'Running…');
    $('message').value = '';
    $('images').value = '';

    const operation = await startTurn(conn(), threadId, input);
    state.operationId = operation.operation_id;
    state.turnId = operation.turn_id;
    $('interrupt').disabled = false;

    // Poll for approvals while the turn runs: a tool prompt can arrive before
    // the first event reaches this page.
    state.pollTimer = setInterval(refreshRequests, 1500);

    for await (const { data } of streamOperation(conn(), operation.operation_id)) {
      renderEvent(data);
      if (data.method === 'bridge/completed' || data.method === 'bridge/error') break;
    }
    say($('send-status'), 'Done.', 'ok');
  } catch (err) {
    say($('send-status'), describe(err), 'error');
    line('error', describe(err));
  } finally {
    clearInterval(state.pollTimer);
    $('interrupt').disabled = true;
    refreshRequests();
  }
};

$('interrupt').onclick = async () => {
  const threadId = $('thread-id').value.trim();
  if (!threadId || !state.turnId) return;
  try {
    await interruptTurn(conn(), threadId, state.turnId);
    line('meta', '■ interrupted');
  } catch (err) {
    line('error', describe(err));
  }
};

// A blank base URL means "this origin", which is the common case when the page
// is served by the bridge itself.
$('connect').click();
