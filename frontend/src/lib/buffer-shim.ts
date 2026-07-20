// @meshsdk/wallet's browser bundle assumes several Node globals (Buffer,
// global, process) that Vite doesn't polyfill for the browser by default --
// it references them as bare identifiers, which throws ReferenceError at
// module-eval time rather than failing gracefully. Import this before
// anything that touches @meshsdk/wallet -- and only from client-side code,
// it must never run during SSR.
import { Buffer } from "buffer";
import process from "process";

if (typeof window !== "undefined") {
  const w = window as unknown as { Buffer?: unknown; global?: unknown; process?: unknown };
  if (!w.Buffer) w.Buffer = Buffer;
  if (!w.global) w.global = window;
  if (!w.process) w.process = process;
}
