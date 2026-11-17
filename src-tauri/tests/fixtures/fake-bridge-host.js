// A fake bridge host for the offline account tests (selected with
// MOTHER_CLAUDE_BRIDGE_HOST_PATH). It speaks the host's line protocol just
// enough: `ready` first, then a `reply` to every request. Each request is
// appended to $FAKE_BRIDGE_LOG with the account part of its environment — the
// variable names, the config directory and only the SHA-256 of a token.
//
// Its session store is the conversations created through it, each in the
// config directory it was created with (`store.list` with a `configDir`
// lists those, and the transcripts a test wrote into that directory's
// `projects`; without one the rest). A turn whose text says "ASK" raises a
// tool request first and finishes once it is answered (`request.respond`).
// Closing an open conversation ends its stream: a `session` frame follows the
// reply, as with the real host.
//
// A bare conversation (the Claude API's, `options.bare`) answers with the
// API's own stream events (`stream_event`), as Claude Code does with partial
// messages on: "Echo (turn N): <the last line of the message's text>", with
// ", seeded" when the message carries a history. "TOOL" calls the first client
// tool (a `client_tool` request, the answer streamed after its result);
// "STOPTEST" says "alpha STOP beta" and keeps its turn open until it is
// interrupted; "AUTHFAIL" answers as a signed-out Claude Code does. With an
// `outputFormat` it answers as Claude Code 2.1.326 does, through its
// StructuredOutput tool: `{"reply": "Echo (turn N): <line>"}`, the tool's
// result before the message's end; "SORETRY" first sends an output the
// schema refuses (tried again), "PREAMBLE" writes a sentence before the call.
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const readline = require('node:readline');

const LOG = process.env.FAKE_BRIDGE_LOG;
const send = (frame) => process.stdout.write(JSON.stringify(frame) + '\n');
const record = (entry) => {
  if (LOG) fs.appendFileSync(LOG, JSON.stringify(entry) + '\n');
};
const sha = (s) => crypto.createHash('sha256').update(s).digest('hex');
const list = (dir) => {
  try {
    return fs.readdirSync(dir);
  } catch {
    return [];
  }
};

// sessionId -> { configDir, lastModified }
const store = new Map();
// Conversations open now. Closing one ends its stream, which the host reports
// with a `session` frame after the reply, as the real host does.
const live = new Set();
// requestId -> { sessionId, turnId }
const asked = new Map();
// sessionId -> { options, turns, turnId } for bare conversations.
const bare = new Map();
// requestId -> { sessionId, turnId, tool } for client tool calls.
const toolCalls = new Map();

const lastLine = (content) => {
  const blocks = Array.isArray(content) ? content : [{ type: 'text', text: String(content || '') }];
  const texts = blocks.filter((b) => b && b.type === 'text').map((b) => b.text || '');
  const all = texts.join('\n');
  const lines = all.split('\n').filter((l) => l.trim());
  return { line: lines.length ? lines[lines.length - 1] : '', all };
};

// One API message as Claude Code passes it on: its stream events, then the
// whole message.
const apiMessage = (sessionId, turnId, blocks, stopReason, between) => {
  const ev = (event) =>
    send({
      t: 'event',
      sessionId,
      turnId,
      message: { type: 'stream_event', event, parent_tool_use_id: null, session_id: sessionId },
    });
  const id = 'msg_fake' + crypto.randomUUID().replace(/-/g, '').slice(0, 16);
  const model = (bare.get(sessionId) || {}).options?.model || 'claude-fake';
  ev({
    type: 'message_start',
    message: {
      id, type: 'message', role: 'assistant', model, content: [],
      stop_reason: null, stop_sequence: null,
      usage: { input_tokens: 10, output_tokens: 1, cache_read_input_tokens: 0 },
    },
  });
  blocks.forEach((b, index) => {
    if (b.type === 'text') {
      ev({ type: 'content_block_start', index, content_block: { type: 'text', text: '' } });
      const half = Math.ceil(b.text.length / 2);
      for (const part of [b.text.slice(0, half), b.text.slice(half)]) {
        if (part) ev({ type: 'content_block_delta', index, delta: { type: 'text_delta', text: part } });
      }
    } else {
      ev({ type: 'content_block_start', index, content_block: { type: 'tool_use', id: b.id, name: b.name, input: {} } });
      ev({ type: 'content_block_delta', index, delta: { type: 'input_json_delta', partial_json: JSON.stringify(b.input) } });
    }
    ev({ type: 'content_block_stop', index });
  });
  const assistant = () =>
    send({ t: 'event', sessionId, turnId, message: { type: 'assistant', message: { id, content: blocks } } });
  // A tool Claude Code runs itself answers before the message's end.
  if (between) {
    assistant();
    between();
  }
  if (stopReason) {
    ev({ type: 'message_delta', delta: { stop_reason: stopReason, stop_sequence: null }, usage: { output_tokens: 5 } });
    ev({ type: 'message_stop' });
  }
  if (!between) assistant();
};

// An answer through Claude Code's StructuredOutput tool.
const structuredTurn = (c, conv, line, all) => {
  const output = { reply: `Echo (turn ${conv.turns}): ${line}` };
  const attempt = (input, taken, preamble) => {
    const toolUseId = 'toolu_so' + crypto.randomUUID().replace(/-/g, '').slice(0, 8);
    const blocks = [];
    if (preamble) blocks.push({ type: 'text', text: 'Here is the JSON.' });
    blocks.push({ type: 'tool_use', id: toolUseId, name: 'StructuredOutput', input });
    apiMessage(c.sessionId, c.turnId, blocks, 'tool_use', () =>
      send({
        t: 'event', sessionId: c.sessionId, turnId: c.turnId,
        message: {
          type: 'user', parent_tool_use_id: null, session_id: c.sessionId,
          message: { role: 'user', content: [taken
            ? { type: 'tool_result', tool_use_id: toolUseId, content: 'Structured output provided successfully' }
            : { type: 'tool_result', tool_use_id: toolUseId, is_error: true,
                content: 'Output does not match required schema: must have required property \'reply\'' }] },
        },
      }),
    );
  };
  if (all.includes('SORETRY')) attempt({ answer: 'wrong shape' }, false, false);
  attempt(output, true, all.includes('PREAMBLE'));
  send({
    t: 'event', sessionId: c.sessionId, turnId: c.turnId,
    message: { type: 'result', subtype: 'success', is_error: false, result: JSON.stringify(output), structured_output: output },
  });
  send({ t: 'turn', sessionId: c.sessionId, turnId: c.turnId, status: 'completed' });
};

const bareTurn = (c) => {
  const conv = bare.get(c.sessionId);
  conv.turns += 1;
  conv.turnId = c.turnId;
  const { line, all } = lastLine(c.content);
  if (all.includes('AUTHFAIL')) {
    // Claude Code signed out: a made-up assistant message, no API events.
    const event = (message) => send({ t: 'event', sessionId: c.sessionId, turnId: c.turnId, message });
    event({
      type: 'assistant',
      error: 'authentication_failed',
      message: { content: [{ type: 'text', text: 'Not logged in · Please run /login' }] },
    });
    event({ type: 'result', subtype: 'success', is_error: true, result: 'Not logged in · Please run /login' });
    send({ t: 'turn', sessionId: c.sessionId, turnId: c.turnId, status: 'failed',
           error: { code: 'claude_login_required', message: 'Not logged in · Please run /login' } });
    return;
  }
  if (conv.options.outputFormat) {
    structuredTurn(c, conv, line, all);
    return;
  }
  const tools = conv.options.clientTools || [];
  if (all.includes('TOOL') && tools.length) {
    const toolUseId = 'toolu_fake' + conv.turns;
    apiMessage(c.sessionId, c.turnId, [
      { type: 'tool_use', id: toolUseId, name: 'mcp__client__' + tools[0].name, input: { city: 'Paris' } },
    ], 'tool_use');
    const requestId = crypto.randomUUID();
    toolCalls.set(requestId, { sessionId: c.sessionId, turnId: c.turnId, toolUseId });
    send({
      t: 'request', requestId, sessionId: c.sessionId, turnId: c.turnId,
      kind: 'client_tool', tool: tools[0].name, input: { city: 'Paris' },
    });
    return;
  }
  if (all.includes('STOPTEST')) {
    apiMessage(c.sessionId, c.turnId, [{ type: 'text', text: 'alpha STOP beta' }], 'end_turn');
    return; // open until interrupted
  }
  const seeded = all.includes('<conversation_so_far>') ? ', seeded' : '';
  apiMessage(c.sessionId, c.turnId, [{ type: 'text', text: `Echo (turn ${conv.turns}${seeded}): ${line}` }], 'end_turn');
  finish(c.sessionId, c.turnId);
};

const finish = (sessionId, turnId) => {
  send({
    t: 'event',
    sessionId,
    turnId,
    message: { type: 'result', subtype: 'success', is_error: false, result: 'ok' },
  });
  send({ t: 'turn', sessionId, turnId, status: 'completed' });
};

send({ t: 'ready', pid: process.pid });
readline.createInterface({ input: process.stdin }).on('line', (line) => {
  let c;
  try {
    c = JSON.parse(line);
  } catch {
    return;
  }
  const env = c.options && c.options.env;
  const set = (env && env.set) || {};
  record({
    op: c.op,
    sessionId: c.sessionId || null,
    configDir: c.configDir || null,
    resume: (c.options && c.options.resume) || null,
    forkSession: Boolean(c.options && c.options.forkSession),
    model: (c.options && c.options.model) || null,
    permissionMode: (c.options && c.options.permissionMode) || null,
    systemPromptAppend: (c.options && c.options.systemPromptAppend) || null,
    bare: Boolean(c.options && c.options.bare),
    systemPrompt: c.options && 'systemPrompt' in c.options ? c.options.systemPrompt : null,
    clientTools: (c.options && c.options.clientTools && c.options.clientTools.map((t) => t.name)) || null,
    maxOutputTokens: (c.options && c.options.maxOutputTokens) || null,
    thinkingConfig: (c.options && c.options.thinkingConfig) || null,
    outputFormat: (c.options && c.options.outputFormat) || null,
    env: env
      ? {
          set: Object.keys(set).sort(),
          unset: env.unset || [],
          configDir: set.CLAUDE_CONFIG_DIR || null,
          tokenSha256: set.CLAUDE_CODE_OAUTH_TOKEN ? sha(set.CLAUDE_CODE_OAUTH_TOKEN) : null,
        }
      : null,
  });
  const reply = (result) => send({ t: 'reply', id: c.id, ok: true, result });
  switch (c.op) {
    case 'session.create':
      if (c.options && c.options.bare) {
        const from = c.options.resume && bare.get(c.options.resume);
        bare.set(c.sessionId, { options: c.options, turns: from ? from.turns : 0, turnId: null });
      }
      store.set(c.sessionId, {
        configDir: set.CLAUDE_CONFIG_DIR || null,
        lastModified: Date.now() + store.size,
      });
      live.add(c.sessionId);
      return reply({ sessionId: c.sessionId, cwd: c.options && c.options.cwd });
    case 'session.close':
      if (!live.has(c.sessionId)) return reply({ ok: true, alreadyClosed: true });
      live.delete(c.sessionId);
      reply({ ok: true });
      return send({ t: 'session', sessionId: c.sessionId, state: 'closed', reason: 'stream ended' });
    case 'store.list': {
      const dir = c.configDir || null;
      const sessions = [...store.entries()]
        .filter(([, s]) => s.configDir === dir)
        .map(([sessionId, s]) => ({ sessionId, summary: 'fake', lastModified: s.lastModified }));
      if (dir) {
        const projects = path.join(dir, 'projects');
        for (const project of list(projects)) {
          for (const file of list(path.join(projects, project))) {
            const id = file.endsWith('.jsonl') ? file.slice(0, -6) : null;
            if (id && !store.has(id)) {
              sessions.push({ sessionId: id, summary: 'on disk', lastModified: 1 });
            }
          }
        }
      }
      return reply({ sessions });
    }
    case 'session.input': {
      reply({ turnId: c.turnId });
      if (bare.has(c.sessionId)) return bareTurn(c);
      // A turn: "AUTHFAIL" in the text answers like an expired token, "ASK"
      // waits on a tool approval, anything else succeeds.
      const text = JSON.stringify(c.content || '');
      const event = (message) =>
        send({ t: 'event', sessionId: c.sessionId, turnId: c.turnId, message });
      if (text.includes('AUTHFAIL')) {
        event({ type: 'assistant', error: 'authentication_failed', message: { content: [] } });
        event({ type: 'result', subtype: 'error_during_execution', is_error: true });
        send({ t: 'turn', sessionId: c.sessionId, turnId: c.turnId, status: 'completed' });
      } else if (text.includes('ASK')) {
        const requestId = crypto.randomUUID();
        asked.set(requestId, { sessionId: c.sessionId, turnId: c.turnId });
        send({
          t: 'request',
          requestId,
          sessionId: c.sessionId,
          turnId: c.turnId,
          kind: 'permission',
          tool: 'Bash',
          prompt: 'Run ls',
          input: { command: 'ls' },
        });
      } else {
        finish(c.sessionId, c.turnId);
      }
      return;
    }
    case 'session.interrupt': {
      const conv = bare.get(c.sessionId);
      const turnId = conv && conv.turnId;
      if (conv) conv.turnId = null;
      reply({ sessionId: c.sessionId, turnId: turnId || null });
      if (turnId) send({ t: 'turn', sessionId: c.sessionId, turnId, status: 'interrupted' });
      return;
    }
    case 'request.respond': {
      reply({ ok: true });
      const call = toolCalls.get(c.requestId);
      if (call) {
        toolCalls.delete(c.requestId);
        const result = c.result || {};
        const text = Array.isArray(result.content)
          ? result.content.map((b) => b.text || '').join('')
          : String(result.content || '');
        send({
          t: 'event', sessionId: call.sessionId, turnId: call.turnId,
          message: { type: 'user', message: { role: 'user', content: [{ type: 'tool_result', tool_use_id: call.toolUseId, content: text }] } },
        });
        apiMessage(call.sessionId, call.turnId, [
          { type: 'text', text: `It is ${result.is_error ? 'unknown' : 'known'} in Paris: ${text}` },
        ], 'end_turn');
        finish(call.sessionId, call.turnId);
        return;
      }
      const waiting = asked.get(c.requestId);
      if (waiting) {
        asked.delete(c.requestId);
        finish(waiting.sessionId, waiting.turnId);
      }
      return;
    }
    default:
      return reply({ ok: true });
  }
});
