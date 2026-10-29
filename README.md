# Funding Swap

> **Status:** This code is unaudited and untested.

Funding Swap lets traders swap fixed funding for floating funding on Solana. A
position pays or receives a fixed annualized rate while the opposite leg tracks
the funding or carry actually charged by a venue over the market's duration.

Phoenix Rise supplies a signed cumulative funding checkpoint. Funding Swap
weights that quote-denominated funding by price to obtain the settlement value
used for user size. The trader who pays fixed receives the floating USDC
settlement over that period.

Jupiter Perps supplies a continuous borrow/lock carry rate. The same fixed-vs-
floating swap is opened against that source for the market's duration.

USDC is the single collateral and settlement asset. Different markets may use
different underlyings.
The design is inspired by Boros. The whitepaper is available at
https://github.com/pendle-finance/boros-core-public/blob/main/whitepapers/Boros.pdf
