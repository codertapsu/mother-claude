import type { ErrorCode, WireError } from './protocol.ts';

/** A command failure with a protocol error code. */
export class CommandError extends Error {
  readonly code: ErrorCode;
  readonly data: unknown;

  constructor(code: ErrorCode, message: string, data?: unknown) {
    super(message);
    this.name = 'CommandError';
    this.code = code;
    this.data = data;
  }

  toWire(): WireError {
    const wire: WireError = { code: this.code, message: this.message };
    if (this.data !== undefined) wire.data = this.data;
    return wire;
  }
}

export function permissionNeeded(permission: string, message: string): CommandError {
  return new CommandError('permission_needed', message, { permission });
}

/** A plain error as returned from the content script or a chrome API. */
export interface ErrorShape {
  code: ErrorCode;
  message: string;
  data?: unknown;
}

export function isErrorShape(v: unknown): v is ErrorShape {
  return (
    typeof v === 'object' &&
    v !== null &&
    typeof (v as { code?: unknown }).code === 'string' &&
    typeof (v as { message?: unknown }).message === 'string'
  );
}

export function messageOf(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === 'string') return e;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}
