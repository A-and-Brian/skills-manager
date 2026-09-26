/**
 * A fake Tauri backend for the E2E tests. `src/main.tsx` imports it in the
 * "e2e" Vite mode only, before the app renders.
 *
 * Every invoke is answered from in-memory state by `handlers`. Tests start it
 * from `window.__E2E_SEED__` and drive it through `window.__e2e` (see
 * `e2e/fixtures.ts`). The state is kept in sessionStorage, so it survives a
 * page reload the way the real backend's data would.
 */
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import { handlers } from "./handlers";
import { createState, type Seed, type State } from "./state";

export interface RecordedCall {
  cmd: string;
  args: unknown;
}

/** The test's handle on the fake backend. */
export interface E2EControl {
  /** Every invoke of this page load, in order. */
  calls: RecordedCall[];
  /** The next `cmd` rejects with `message` instead of running. */
  failNext(cmd: string, message: string): void;
  /** Calls to `cmd` wait until `release(cmd)`. */
  hold(cmd: string): void;
  release(cmd: string): void;
  /** Emit a backend event to the app's listeners. */
  emit(event: string, payload: unknown): Promise<void>;
  /** Replace top-level parts of the state, as if the backend changed on its own. */
  patch(partial: Partial<State>): void;
}

declare global {
  interface Window {
    __E2E_SEED__?: Seed;
    __e2e?: E2EControl;
  }
}

const STORAGE_KEY = "e2e:fake-backend-state";

function loadState(): State {
  const saved = sessionStorage.getItem(STORAGE_KEY);
  return saved ? (JSON.parse(saved) as State) : createState(window.__E2E_SEED__ ?? {});
}

const state = loadState();
const save = () => sessionStorage.setItem(STORAGE_KEY, JSON.stringify(state));
save();

const calls: RecordedCall[] = [];
const failures = new Map<string, string>();
const holds = new Map<string, { promise: Promise<void>; resolve: () => void }>();

mockWindows("main");
mockIPC(
  async (cmd, args) => {
    calls.push({ cmd, args });
    await holds.get(cmd)?.promise;

    const failure = failures.get(cmd);
    if (failure !== undefined) {
      failures.delete(cmd);
      throw failure;
    }

    const handler = handlers[cmd] as ((args: unknown, state: State) => unknown) | undefined;
    if (!handler) {
      const message = `fake backend: no handler for ${cmd}`;
      // The fixture fails the test on this line, even when the app swallows the error.
      console.error(message);
      throw new Error(message);
    }
    const result = handler(args ?? {}, state);
    save();
    // A copy, so the app never holds a reference into the fake's state.
    return result === undefined ? null : structuredClone(result);
  },
  { shouldMockEvents: true },
);

window.__e2e = {
  calls,
  failNext: (cmd, message) => failures.set(cmd, message),
  hold: (cmd) => {
    let resolve = () => {};
    const promise = new Promise<void>((r) => (resolve = r));
    holds.set(cmd, { promise, resolve });
  },
  release: (cmd) => {
    holds.get(cmd)?.resolve();
    holds.delete(cmd);
  },
  emit: (event, payload) => emit(event, payload),
  patch: (partial) => {
    Object.assign(state, partial);
    save();
  },
};
