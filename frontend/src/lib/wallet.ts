import type { BrowserWallet } from "@meshsdk/wallet";
import type { UtxoRefInput } from "./api";

export interface SelectedUtxo extends UtxoRefInput {
  lovelace: number;
}

/** Largest pure-ADA (no native tokens) UTxO in the wallet -- used to pay
 * the fee and receive change. The offchain API expects the caller to have
 * already picked this (see offchain/README.md: "this server doesn't do
 * coin selection"). */
export async function pickFeeInput(wallet: BrowserWallet): Promise<SelectedUtxo> {
  const utxos = await wallet.getUtxos();
  const pureAda = utxos.filter((u) => u.output.amount.length === 1 && u.output.amount[0].unit === "lovelace");
  if (pureAda.length === 0) {
    throw new Error("wallet has no pure-ADA UTxO to use as a fee input");
  }
  const best = pureAda.reduce((a, b) => (Number(a.output.amount[0].quantity) > Number(b.output.amount[0].quantity) ? a : b));
  return {
    tx_hash: best.input.txHash,
    output_index: best.input.outputIndex,
    lovelace: Number(best.output.amount[0].quantity),
  };
}

/** Smallest pure-ADA UTxO in the wallet (other than `exclude`) to use as the
 * collateral input. The offchain API only needs a `{tx_hash, output_index}`
 * ref for this (see api.ts UtxoRefInput) -- it doesn't need the wallet's
 * own "reserved collateral" setting at all.
 *
 * Deliberately not `wallet.getCollateral()` / CIP-30's dedicated collateral
 * API: that call is legacy (pre-CIP-40 collateral-return), wallets differ
 * on whether the required `amount` param filters by minimum size or is
 * ignored, and Lace's CIP-30 shim only ever populates the `experimental`
 * fallback path -- three different ways to get a false "no collateral set"
 * even when the wallet has ADA to spare. Since any pure-ADA UTxO is valid
 * collateral post-CIP-40, picking one ourselves (same approach as
 * `pickFeeInput`) sidesteps all of that permanently. */
export async function pickCollateral(wallet: BrowserWallet, exclude?: UtxoRefInput): Promise<UtxoRefInput> {
  const utxos = await wallet.getUtxos();
  const pureAda = utxos.filter(
    (u) =>
      u.output.amount.length === 1 &&
      u.output.amount[0].unit === "lovelace" &&
      !(exclude && u.input.txHash === exclude.tx_hash && u.input.outputIndex === exclude.output_index),
  );
  if (pureAda.length === 0) {
    throw new Error("wallet has no pure-ADA UTxO available to use as collateral");
  }
  const best = pureAda.reduce((a, b) => (Number(a.output.amount[0].quantity) < Number(b.output.amount[0].quantity) ? a : b));
  return {
    tx_hash: best.input.txHash,
    output_index: best.input.outputIndex,
  };
}

/** The wallet's current active address. `getChangeAddress()` is always
 * defined regardless of transaction history, unlike `getUsedAddresses()`
 * (empty for a freshly-selected account that's never sent/received
 * anything) -- important since this also drives account-switch polling,
 * where the newly-selected account may have zero history. */
export async function currentAddress(wallet: BrowserWallet): Promise<string> {
  try {
    return await wallet.getChangeAddress();
  } catch {
    const used = await wallet.getUsedAddresses();
    if (used.length > 0) return used[0];
    throw new Error("wallet has no change address or used addresses");
  }
}
