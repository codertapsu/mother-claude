# Machines — one Mother Claude across your computers

Pair your computers once, then see and drive every computer's Claude Code
sessions from any of them — and from a phone paired with any of them. The
design and the plan for what comes next (moving a session with its code,
copies, work on another computer) are in
[PLAN-MACHINES.md](PLAN-MACHINES.md).

> **Status**: Phases 1 (pairing and setup) and 2 (seeing and driving other
> computers' sessions) are built. Moving sessions comes next.

## Turning it on

Machines is off until you set it up: **Machines › Set up Machines** on the
desktop. The setup runs on each computer — only a computer can check its
own operating system — and once two are paired, each shows every computer's
checklist side by side. Machines turns on only when no *required* check is
left; the checks keep running afterwards as **Machines health** (a red dot
on the sidebar entry when something breaks again).

The seven steps:

1. **This computer** — its checks (below), each with what fixes it.
2. **Your machines** — pair the others (next section).
3. **Every machine, side by side** — a fix that needs a person at another
   computer reads *On DESKTOP-PC:*; *Ask DESKTOP-PC to fix* shows it there as
   a notice. Nothing on a computer changes because another asked.
4. **What each machine may do** — the permissions, per computer.
5. **Accounts and folders** — comes with moving sessions.
6. **Options** — the relay and keeping awake.
7. **Test, then turn on** — a connection and a 20 MB transfer between this
   computer and each other one, then *Turn on Machines*.

### What is checked

*Required* blocks Machines; *recommended* works, but worse.

| Check | Why | Fix |
|---|---|---|
| Mother Claude | every computer must speak the same Machines protocol | update it there |
| Stays awake while used | a computer asleep cannot answer | *Keep awake while used*: the app holds a power assertion while another computer is connected — no system setting changes |
| **macOS:** Local Network access | without it macOS refuses Mother Claude your local network, and the Mac reaches the others only through the relay, even at home | allow it when macOS asks, or *System Settings › Privacy & Security › Local Network* |
| **macOS:** the firewall | if it blocks Mother Claude, the others reach this Mac only through the relay | *System Settings › Network › Firewall* |
| **Windows:** the network type | on a *Public* network Windows blocks incoming connections | set your home or office network to *Private* |
| **Windows:** the firewall | without a rule, the others reach this PC only through the relay | *Allow through the firewall*: one rule for Mother Claude, after Windows' administrator prompt |
| The relay | across networks your computers meet through it | check its address, or pick the public relays |
| Paired computers | Machines works between computers you paired | *Pair a machine* |
| Each paired computer | the same protocol and relay setting on both | update it / *Use this setting everywhere* |

The network checks are *required* only with the **No relay** setting: with a
relay, two computers still meet — through it, slower.

## Pairing

On the first computer, **Machines › Pair a machine** shows an 8-character
code (it works once, for five minutes). On the other, **Machines › Join**:
pick it from *Nearby* (same network) or type the code — it works across the
internet too. Both screens then show the **same six digits**: check they
match, choose on each computer what the other may do on it, and confirm on
both. When the other computer joined over the internet, the first one asks
you to *type* the number the other shows, as phone pairing does.

What the code and the digits protect against: the code finds the other
computer, and only someone holding it can start a pairing; the digits are
computed from both computers' keys, so a computer in the middle — even one
that saw the code — would make the two screens show different numbers.
Nothing is stored until both sides confirm.

A paired computer is recognised by its key alone, kept in your Keychain or
Windows Credential Manager. Reinstalling Mother Claude makes a new key: pair
that computer again.

### What another computer may do here

| Permission | Lets it | Default |
|---|---|---|
| **See** | list sessions, read transcripts, see approvals and questions | on |
| **Drive** | send messages, answer ordinary approvals, interrupt, stop, set model, effort and mode | on |
| **Move** | move sessions here and away (coming) | on |
| **Run** | run commands and tasks here (later) | off |
| **Dangerous** | bypass permissions, approve dangerous tool calls, send raw keys | off |

Each is set separately for the **same network** (a direct path on your
local network) and the **internet** (everything else). Change them on the
Machines page; *Revoke* forgets a computer and cuts its connections at once.

## Seeing and driving another computer

Once paired, every computer's sessions appear in **Sessions** with a chip
naming their computer (a filter picks one computer); open one and use it as
if it ran here — the message box, approvals, stop, model, effort. A phone
paired with any of your computers sees and drives them all through it.

A dangerous action on another computer needs both its **Dangerous**
permission for this computer *and* your own right here: a phone without the
dangerous permission on the Mac cannot reach for it through the Mac. Every
action another computer takes is in the audit log of the computer it acted
on, with who acted; a dangerous one also shows a notice there.

When a computer goes offline, its sessions stay listed, greyed, until it is
back.

## How it connects

Your computers talk over QUIC between their keys, end to end encrypted:
directly on the same network, hole-punched across networks when the
networks allow it, otherwise through a **relay** that only ever sees
ciphertext. In *Options*:

- **Public relays** (default) — n0's, anywhere, no setup. Measured in the
  spikes: fine for driving sessions, slower for large transfers.
- **Your own relay** — the address of an `iroh-relay` you run (on a NAS, a
  small server). Your computers then ask no outside service anything.
- **No relay** — the same network only.

The setting must be the same on every computer; changing it on one offers
it to the others, and each applies it when you accept there. Nothing runs on
the network until you use Machines: a computer that never pairs makes no new
connection.

## Troubleshooting

- **The other computer is not in *Nearby*.** Type the code instead (it works
  across networks too). On a Mac, check Local Network access for Mother
  Claude.
- **"No computer showing this code was found."** Check the code, and that
  both computers have the internet or share a network; the code lasts five
  minutes.
- **A computer shows *Through the relay* at home.** On the Mac, Local
  Network access; on Windows, the network type and the firewall check.
- **"This computer has not given yours the … permission."** Change it on
  the other computer's Machines page.
