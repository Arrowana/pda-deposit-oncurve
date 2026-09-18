# On-curve deposits for a PDA wallet

**Goal:** Give an off-curve smart-wallet PDA `P` one on-curve deposit address
`D` for SOL, legacy SPL tokens, and Token-2022 tokens.

**How:** Derive a sweeper PDA `S` from `P`, then grind
`D = create_with_seed(S, seed, TOKEN_PROGRAM_ID)` until on-curve. Initialize
`D` as a legacy SPL 1-of-1 multisig whose only member is `S`. Legacy and
Token-2022 assets use separate ATAs, both owned by `D`; Token-2022 accepts a
legacy Token-owned multisig authority.

Anyone can ask the sweeper program to sweep. It signs for `S` only after
verifying that the destination is `P` or the canonical `ATA(P, mint)`, so no
user or smart-wallet interaction is needed and the keeper never has custody.
`P` is only the destination, not an authority. The sweeper program must be
immutable or governed as securely as the wallet, because its PDA is the sole
authority.

**Rent on mainnet-beta (18 September 2026):** `D` (355 bytes) requires
2,453,640 lamports (**0.00245364 SOL**). This is a rent-exempt reserve, not a
transaction fee.

**Lazy creation:** An ATA owned by `D` can receive tokens before `D` exists.
On the first sweep, the keeper pays the rent and calls the sweeper program,
which signs for base `S`, creates `D`, initializes the multisig, and sweeps
atomically.

If the first deposit is only SOL, `D` is still a system-owned, zero-data
account. The sweeper can sign for base `S` and use `TransferWithSeed` with
`from_owner = TOKEN_PROGRAM_ID` to send the entire balance to `P`. `D` never
needs to be allocated as a multisig, so no rent reserve is funded or left
behind; only the transaction fee is paid. If token sweeping is also required
while SOL remains at `D`, drain the SOL first or fund the rent deficit and use
`AllocateWithSeed` before initializing the multisig.

The LiteSVM test currently exercises the legacy-token path and uses
`with_sigverify(false)` to stand in for the real sweeper program's
`invoke_signed` sweeper-PDA signature.

```sh
cargo test --test oncurve_deposit -- --nocapture
```
