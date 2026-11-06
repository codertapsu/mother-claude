// Launches the installed Chrome with a throwaway profile and drives it over
// the DevTools protocol on a pipe (--remote-debugging-pipe: fd 3 in, fd 4
// out, NUL-separated JSON). Branded Chrome 137+ ignores --load-extension, so
// the unpacked build is loaded with Extensions.loadUnpacked, which needs the
// pipe and --enable-unsafe-extension-debugging. Never the user's profile.

import { spawn, type ChildProcess } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { Readable, Writable } from 'node:stream';

export interface TargetInfo {
  targetId: string;
  type: string;
  url: string;
  title: string;
}

interface Message {
  id?: number;
  method?: string;
  params?: Record<string, unknown>;
  result?: Record<string, unknown>;
  error?: { message: string };
  sessionId?: string;
}

export function defaultBrowserPath(): string | null {
  const fromEnv = process.env['MC_E2E_BROWSER'];
  if (fromEnv) return existsSync(fromEnv) ? fromEnv : null;
  const candidates =
    process.platform === 'darwin'
      ? ['/Applications/Google Chrome.app/Contents/MacOS/Google Chrome']
      : process.platform === 'win32'
        ? [
            'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
            'C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe',
          ]
        : ['/usr/bin/google-chrome', '/usr/bin/google-chrome-stable', '/usr/bin/chromium'];
  return candidates.find((p) => existsSync(p)) ?? null;
}

export class Browser {
  private seq = 0;
  private buffer = Buffer.alloc(0);
  private readonly pending = new Map<
    number,
    { resolve(v: Record<string, unknown>): void; reject(e: Error): void; method: string }
  >();
  private readonly listeners = new Set<(m: Message) => void>();
  readonly stderr: string[] = [];
  readonly proc: ChildProcess;
  readonly userDataDir: string;
  private readonly input: Writable;

  private constructor(proc: ChildProcess, input: Writable, output: Readable, userDataDir: string) {
    this.proc = proc;
    this.input = input;
    this.userDataDir = userDataDir;
    output.on('data', (chunk: Buffer) => this.onData(chunk));
    proc.stderr?.on('data', (d: Buffer) => {
      this.stderr.push(String(d));
      if (this.stderr.length > 200) this.stderr.shift();
    });
  }

  static async launch(executable: string, opts: { headless?: boolean } = {}): Promise<Browser> {
    const userDataDir = await mkdtemp(join(tmpdir(), 'mc-ext-e2e-'));
    const args = [
      '--remote-debugging-pipe',
      '--enable-unsafe-extension-debugging',
      `--user-data-dir=${userDataDir}`,
      '--no-first-run',
      '--no-default-browser-check',
      '--disable-sync',
      '--disable-background-networking',
      '--disable-component-update',
      '--disable-default-apps',
      '--disable-background-timer-throttling',
      '--disable-backgrounding-occluded-windows',
      '--disable-renderer-backgrounding',
      '--disable-features=Translate,OptimizationHints,MediaRouter,DialMediaRouteProvider,AutofillServerCommunication,CertificateTransparencyComponentUpdater',
      // Never the real Keychain or password store.
      '--use-mock-keychain',
      '--password-store=basic',
      '--window-size=1280,900',
      ...(opts.headless === false ? [] : ['--headless']),
      'about:blank',
    ];
    const proc = spawn(executable, args, { stdio: ['ignore', 'ignore', 'pipe', 'pipe', 'pipe'] });
    const input = proc.stdio[3] as Writable;
    const output = proc.stdio[4] as Readable;
    const browser = new Browser(proc, input, output, userDataDir);
    await browser.send('Browser.getVersion');
    return browser;
  }

  private onData(chunk: Buffer): void {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    let end: number;
    while ((end = this.buffer.indexOf(0)) >= 0) {
      const text = this.buffer.subarray(0, end).toString('utf8');
      this.buffer = this.buffer.subarray(end + 1);
      const msg = JSON.parse(text) as Message;
      if (msg.id !== undefined && this.pending.has(msg.id)) {
        const p = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        if (msg.error) p?.reject(new Error(`${p.method}: ${msg.error.message}`));
        else p?.resolve(msg.result ?? {});
      } else {
        for (const l of this.listeners) l(msg);
      }
    }
  }

  send<T = Record<string, unknown>>(
    method: string,
    params: Record<string, unknown> = {},
    sessionId?: string,
  ): Promise<T> {
    const id = ++this.seq;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, {
        resolve: resolve as (v: Record<string, unknown>) => void,
        reject,
        method,
      });
      this.input.write(
        `${JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) })}\0`,
      );
    });
  }

  on(listener: (m: Message) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  async loadExtension(path: string): Promise<string> {
    const r = await this.send<{ id: string }>('Extensions.loadUnpacked', { path });
    return r.id;
  }

  async targets(): Promise<TargetInfo[]> {
    const r = await this.send<{ targetInfos: TargetInfo[] }>('Target.getTargets');
    return r.targetInfos;
  }

  async waitForTarget(pred: (t: TargetInfo) => boolean, timeoutMs = 10_000): Promise<TargetInfo> {
    const end = Date.now() + timeoutMs;
    while (Date.now() < end) {
      const t = (await this.targets()).find(pred);
      if (t) return t;
      await new Promise((r) => setTimeout(r, 100));
    }
    throw new Error('target did not appear');
  }

  async attach(targetId: string): Promise<string> {
    const r = await this.send<{ sessionId: string }>('Target.attachToTarget', {
      targetId,
      flatten: true,
    });
    return r.sessionId;
  }

  async detach(sessionId: string): Promise<void> {
    await this.send('Target.detachFromTarget', { sessionId }).catch(() => undefined);
  }

  private readonly contexts = new Map<string, number>();

  /** Attach to a worker and learn its execution context (where `chrome` lives). */
  async attachWorker(targetId: string): Promise<string> {
    const sessionId = await this.attach(targetId);
    const created = new Promise<number>((resolve) => {
      const off = this.on((m) => {
        if (m.sessionId === sessionId && m.method === 'Runtime.executionContextCreated') {
          const ctx = (m.params?.['context'] ?? {}) as { id?: number };
          if (typeof ctx.id === 'number') {
            off();
            resolve(ctx.id);
          }
        }
      });
    });
    await this.send('Runtime.enable', {}, sessionId);
    await this.send('Runtime.runIfWaitingForDebugger', {}, sessionId).catch(() => undefined);
    const id = await Promise.race([
      created,
      new Promise<number>((r) => setTimeout(() => r(-1), 3000)),
    ]);
    if (id >= 0) this.contexts.set(sessionId, id);
    return sessionId;
  }

  /** Evaluate in a target (page or worker) and return the JSON value. */
  async evaluate<T = unknown>(sessionId: string, expression: string): Promise<T> {
    const contextId = this.contexts.get(sessionId);
    const r = await this.send<{
      result: { value?: unknown };
      exceptionDetails?: { exception?: { description?: string }; text?: string };
    }>(
      'Runtime.evaluate',
      {
        expression,
        awaitPromise: true,
        returnByValue: true,
        ...(contextId !== undefined ? { contextId } : {}),
      },
      sessionId,
    );
    if (r.exceptionDetails) {
      throw new Error(
        r.exceptionDetails.exception?.description ?? r.exceptionDetails.text ?? 'evaluate failed',
      );
    }
    return r.result.value as T;
  }

  /** A new tab (not one of the extension's leases). */
  async openPage(url: string): Promise<{ targetId: string; sessionId: string }> {
    const { targetId } = await this.send<{ targetId: string }>('Target.createTarget', { url });
    const sessionId = await this.attach(targetId);
    return { targetId, sessionId };
  }

  async close(): Promise<void> {
    await this.send('Browser.close').catch(() => undefined);
    await new Promise<void>((resolve) => {
      if (this.proc.exitCode !== null) return resolve();
      const t = setTimeout(() => {
        this.proc.kill('SIGKILL');
        resolve();
      }, 5000);
      this.proc.once('exit', () => {
        clearTimeout(t);
        resolve();
      });
    });
    await rm(this.userDataDir, { recursive: true, force: true }).catch(() => undefined);
  }
}
