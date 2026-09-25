import type { MockBackend } from './mock-api';

/** Seam for a dev-only fixture backend; none is bundled yet. */
export async function loadMockBackend(): Promise<MockBackend | null> {
  return null;
}
