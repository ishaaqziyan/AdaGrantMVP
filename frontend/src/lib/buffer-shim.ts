// @meshsdk/wallet assumes Node globals (Buffer/global/process) Vite doesn't polyfill -- import before anything touching @meshsdk/wallet, client-side only (never SSR).
import { Buffer } from "buffer";
import process from "process";

if (typeof window !== "undefined") {
  const w = window as unknown as { Buffer?: unknown; global?: unknown; process?: unknown };
  if (!w.Buffer) w.Buffer = Buffer;
  if (!w.global) w.global = window;
  if (!w.process) w.process = process;
}
