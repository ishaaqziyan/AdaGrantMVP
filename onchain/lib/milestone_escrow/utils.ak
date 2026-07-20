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
