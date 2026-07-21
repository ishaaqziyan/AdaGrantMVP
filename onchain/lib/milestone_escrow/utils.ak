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
