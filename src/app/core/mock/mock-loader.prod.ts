import type { MockBackend } from './mock-api';

/** Production build: there is no mock backend. */
export async function loadMockBackend(): Promise<MockBackend | null> {
  return null;
}
