import type { MockBackend } from './mock-api';

/** Dev build: lazily load the fixture backend (its own chunk). Replaced by
 * mock-loader.prod.ts in production builds. */
export async function loadMockBackend(): Promise<MockBackend | null> {
  const { createMockBackend } = await import('./mock-backend');
  return createMockBackend();
}
