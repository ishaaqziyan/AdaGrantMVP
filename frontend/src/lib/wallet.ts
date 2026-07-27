import type { BrowserWallet } from "@meshsdk/wallet";
import type { UtxoRefInput } from "./api";

export interface SelectedUtxo extends UtxoRefInput {
  lovelace: number;
}

/** Largest pure-ADA UTxO in the wallet -- pays the fee/receives change; the offchain API doesn't do coin selection itself. */
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

/** Smallest pure-ADA UTxO (other than `exclude`) to use as collateral.
 * Prefers `wallet.getUtxos()` over `wallet.getCollateral()`: that CIP-30 call is legacy, inconsistent across wallets (Lace's shim only populates the `experimental` fallback), and can falsely report "no collateral set". Any pure-ADA UTxO is valid collateral post-CIP-40, so we pick one ourselves when we can.
 * Falls back to `getCollateral()` only when that yields nothing: Lace by default reserves a small pure-ADA UTxO purely for collateral and excludes it from `getUtxos()` entirely, so a Lace wallet with only one other UTxO (used as the fee input) would otherwise always fail here even though it has usable collateral. The fallback is purely additive -- if `getCollateral()` also comes back empty (the "falsely reports none" failure mode above), we're no worse off than before. */
export async function pickCollateral(wallet: BrowserWallet, exclude?: UtxoRefInput): Promise<UtxoRefInput> {
  const utxos = await wallet.getUtxos();
  const pureAda = utxos.filter(
    (u) =>
      u.output.amount.length === 1 &&
      u.output.amount[0].unit === "lovelace" &&
      !(exclude && u.input.txHash === exclude.tx_hash && u.input.outputIndex === exclude.output_index),
  );
  if (pureAda.length > 0) {
    const best = pureAda.reduce((a, b) => (Number(a.output.amount[0].quantity) < Number(b.output.amount[0].quantity) ? a : b));
    return {
      tx_hash: best.input.txHash,
      output_index: best.input.outputIndex,
    };
  }

  const reserved = await wallet.getCollateral().catch(() => []);
  const candidate = reserved.find(
    (u) => !(exclude && u.input.txHash === exclude.tx_hash && u.input.outputIndex === exclude.output_index),
  );
  if (!candidate) {
    throw new Error("wallet has no pure-ADA UTxO available to use as collateral");
  }
  return {
    tx_hash: candidate.input.txHash,
    output_index: candidate.input.outputIndex,
  };
}

/** `getChangeAddress()` is always defined, unlike `getUsedAddresses()` (empty for a fresh account) -- matters since this also drives account-switch polling. */
export async function currentAddress(wallet: BrowserWallet): Promise<string> {
  try {
    return await wallet.getChangeAddress();
  } catch {
    const used = await wallet.getUsedAddresses();
    if (used.length > 0) return used[0];
    throw new Error("wallet has no change address or used addresses");
  }
}
