// Shared helpers for `milestone_escrow`: asset naming, a list-update
// helper, tranche-percentage math, and the `tranche_bps`/`approved` shape
// invariant checked on every spend.

use aiken/collection/list
use aiken/primitive/bytearray

/// Asset name for the soulbound Milestone Receipt NFT minted on approval of
/// milestone `index`. One byte is enough for any realistic milestone count
/// (0..255).
pub fn milestone_asset_name(index: Int) -> ByteArray {
  bytearray.concat(#"4d696c6573746f6e65", bytearray.from_int_big_endian(index, 1))
}

/// Replace the element at `index` with `value`, leaving every other
/// element untouched.
pub fn set_at(self: List<a>, index: Int, value: a) -> List<a> {
  self |> list.indexed_map(fn(i, x) { if i == index { value } else { x } })
}

/// Tranche payout for `index`, as a fixed share of the amount originally
/// locked (not the current script balance, which shrinks as tranches pay
/// out).
pub fn tranche_amount(total_locked: Int, tranche_bps: List<Int>, index: Int) -> Int {
  expect Some(bps) = tranche_bps |> list.at(index)
  total_locked * bps / 10000
}

/// `tranche_bps`/`approved` shape invariant the whole percentage-of-total
/// payout model depends on. There's no dedicated "create" redeemer -- the
/// initial lock is just funds sent to the script address, so the validator
/// never runs at creation time. This is checked on every spend instead
/// (the earliest point any Plutus code ever executes for the UTxO), so a
/// hand-crafted malformed datum can sit at the address but can never be
/// successfully approved or released against.
pub fn valid_tranche_bps(
  tranche_bps: List<Int>,
  approved: List<Bool>,
  total_locked: Int,
) -> Bool {
  let n = list.length(tranche_bps)
  let bps_sum = list.foldr(tranche_bps, 0, fn(bps, acc) { bps + acc })
  n > 0 && list.length(approved) == n && list.all(
    tranche_bps,
    fn(bps) { bps > 0 },
  ) && bps_sum == 10_000 && total_locked > 0
}

test valid_tranche_bps_accepts_well_formed_datum() {
  valid_tranche_bps([4000, 3000, 3000], [False, False, False], 100_000_000)
}

test valid_tranche_bps_rejects_zero_entry() {
  // sums to 10000, but a 0% milestone is meaningless
  !valid_tranche_bps([10_000, 0], [False, False], 100_000_000)
}

test valid_tranche_bps_rejects_negative_entry() {
  // sums to 10000, but a negative share breaks the payout math
  !valid_tranche_bps([15_000, -5_000], [False, False], 100_000_000)
}

test valid_tranche_bps_rejects_approved_length_mismatch() {
  !valid_tranche_bps([4000, 3000, 3000], [False, False], 100_000_000)
}

test valid_tranche_bps_rejects_empty_list() {
  !valid_tranche_bps([], [], 100_000_000)
}

test valid_tranche_bps_rejects_non_positive_total_locked() {
  !valid_tranche_bps([4000, 3000, 3000], [False, False, False], 0)
}
