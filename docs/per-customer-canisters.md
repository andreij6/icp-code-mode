# Idea: one canister per customer

Status: idea, not built. Written 2026-11-17.

## The idea

Instead of one shared ICP Code Mode canister that we fund, each customer gets their own
instance, funded with their own cycles. A small **factory canister** creates the instances
and takes a fee. The goal is modest: the service pays for itself and leaves a little over.

## Why this model

The app is single-tenant today. One instance has one allowlist, one on-chain identity (its
principal) and one audit log, and only its controller decides what scripts may call. A team
that wants to point agents at its own canisters needs its own instance anyway.

A private instance gives each customer:

- their own allowlist, limits and bearer tokens,
- their own principal, so target canisters can grant permissions to that customer alone,
- their own audit log,
- their own cycle balance, so their usage never spends anyone else's cycles.

## How it would work

1. The customer pays the factory (ICP or cycles) and calls `create_instance`.
2. The factory creates a new canister with those cycles, minus the fee.
3. The factory installs the ICP Code Mode Wasm, with the customer's first token hash as the
   init argument and an empty allowlist.
4. The factory makes the customer a controller. The customer then manages the allowlist and
   tokens with the existing admin methods.
5. To add cycles later, the customer tops up the instance directly, or through the factory.

## Where the money comes from

- **Setup fee:** a fixed amount kept by the factory when it creates an instance.
- **Top-up margin:** top-ups sent through the factory arrive minus a small cut, for example
  10%. Direct top-ups stay possible, so this only works if the factory adds convenience,
  such as paying in ICP or ckUSDC and getting cycles.
- **Upgrades:** optionally, a fee for upgrading an instance to a new version.

## What an instance costs the customer

Rough figures from the local replica, at about $1.35 per trillion cycles:

- Creating a canister: about 0.5 trillion cycles.
- Idle: about 8 trillion cycles a year, roughly $10. Idle cost follows the memory the
  canister holds (heap and stable memory; about 1.25 GB was measured after use), not the
  12 MB Wasm.
- Per script run: about 15–20 million cycles, a few thousandths of a cent.

## What exists and what is missing

Already in place:

- init arguments for the first token, allowlist and limits,
- controller-only admin methods,
- policy and audit log in stable memory, which survive upgrades,
- Cargo features for smaller per-language builds.

To build:

- The factory canister: payment handling, canister creation, and a registry of instances.
- Chunked install. The Wasm is larger than the 2 MB message limit, so the factory has to
  store it and install it in chunks.
- An upgrade path for instances, and a decision on who may trigger it.
- A page or CLI command for creating an instance and showing its cycle balance.

## Open questions

- **Payment:** accept ICP and convert it to cycles, accept cycles directly, or both?
- **Upgrades and trust:** if the factory stays a co-controller it can ship fixes, but the
  customer has to trust it. If it does not, customers upgrade themselves.
- **Running dry:** how does a customer learn that an instance is low on cycles before it
  freezes?
- **Free trial:** keep one shared public instance, limited to read-only calls on well-known
  canisters, so people can try it before paying?
- **Prices:** the setup fee and margin are not decided.
- **Legal and tax:** selling a service for ICP or cycles may have implications that need
  proper advice.
