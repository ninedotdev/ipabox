#!/usr/bin/env python3
"""Non-interactive remote pair helper.

Replaces `pymobiledevice3 remote pair` which otherwise opens an interactive
menu to pick an IP (problematic when stdout is piped). We browse via Bonjour,
filter by device name, prefer the IPv4 address, and autopair.
"""
from __future__ import annotations
import asyncio
import builtins
import sys

# Flush interactive prompts on their own line so the parent (Go subprocess with
# line-based scanner) can render the prompt in real time.
_orig_input = builtins.input
def _line_input(prompt: str = "") -> str:
    if prompt:
        sys.stdout.write(f"{prompt}\n")
        sys.stdout.flush()
    return _orig_input()
builtins.input = _line_input

from pymobiledevice3.cli.remote import browse_remotepairing_manual_pairing
from pymobiledevice3.exceptions import ConnectionTerminatedError, RemotePairingCompletedError
from pymobiledevice3.remote.tunnel_service import RemotePairingManualPairingService


def _score(ip: str) -> int:
    # Prefer IPv4 addresses, then IPv6 non-link-local, then link-local.
    if "." in ip and ":" not in ip:
        return 0
    if ip.startswith("fe80") or "%" in ip:
        return 2
    return 1


async def pair(ident: str, ip: str, port: int, display: str) -> int:
    print(f"pairing with {display} at {ip}:{port}", flush=True)
    print("accept the trust prompt on the device now…", flush=True)
    try:
        async with RemotePairingManualPairingService(ident, ip, port) as svc:
            await svc.connect(autopair=True)
    except RemotePairingCompletedError:
        # Not an error — the device closes the pairing session right after
        # success. Record was written to ~/.pymobiledevice3/.
        pass
    print("SUCCESS: paired and pair record saved", flush=True)
    return 0


async def main(name: str, ip: str = "", port: int = 0) -> int:
    # The caller already found the device in the manual-pairing advert, so use
    # its address directly — no browse needed, and no second chance to miss it.
    if ip and port:
        try:
            return await pair(name, ip, port, name)
        except PermissionError:
            # Local file error after a successful handshake — re-pairing would
            # only ask for a new PIN and fail the same way.
            raise
        except (ConnectionTerminatedError, OSError) as exc:
            # A stale address (the device left pairing mode and came back on a
            # new port) closes immediately. Fall back to a fresh browse.
            print(f"direct connect failed ({type(exc).__name__}); browsing instead…", flush=True)

    print(f"browsing for {name or '<any>'} (up to 10s)…", flush=True)
    # Retry with increasing timeout — the first mDNS browse after a process
    # starts often misses early announcements.
    answers = []
    for timeout in (3, 5, 10):
        answers = await browse_remotepairing_manual_pairing(timeout=timeout)
        if answers:
            print(f"found {len(answers)} advertisement(s) with timeout={timeout}s", flush=True)
            break
        print(f"  no results within {timeout}s, retrying…", flush=True)

    if not answers:
        print("ERROR: nothing is advertising _remotepairing-manual-pairing._tcp", flush=True)
        print("This is a different service from the _remotepairing._tcp advert used to LIST", flush=True)
        print("devices: it exists only while the device is actually in pairing mode.", flush=True)
        print("  Apple TV: Settings → Remotes and Devices → Remote App and Devices (keep it open)", flush=True)
        print("  iPhone/iPad: pair over USB instead — connect the cable and tap Trust.", flush=True)
        return 2

    candidates = []
    for answer in answers:
        ident = answer.properties.get("identifier", "")
        current = answer.properties.get("name", "")
        # tvOS/iOS _remotepairing adverts almost never carry a "name" TXT key —
        # the only stable selector is the identifier. Match the requested
        # selector against either the friendly name (when present) or the
        # identifier. An empty selector accepts every advert.
        # On the direct-connect fallback the selector may be stale, so also
        # accept the advert that is still on the address we were given.
        same_host = bool(ip) and any(a.full_ip == ip for a in answer.addresses)
        if name and name != current and name != ident and not same_host:
            continue
        for addr in answer.addresses:
            candidates.append(
                (
                    ident,
                    addr.full_ip,
                    answer.port,
                    current or ident,
                )
            )

    if not candidates:
        print(f"ERROR: device '{name}' not found among _remotepairing advertisements", flush=True)
        print("Open Settings → Remotes and Devices → Remote App and Devices on the Apple TV and keep it visible.", flush=True)
        return 3

    # Deduplicate by (identifier, ip) and pick the best address.
    seen = set()
    unique = []
    for ident, ip, port, dn in candidates:
        key = (ident, ip)
        if key in seen:
            continue
        seen.add(key)
        unique.append((ident, ip, port, dn))
    unique.sort(key=lambda c: _score(c[1]))

    ident, ip, port, dn = unique[0]
    return await pair(ident, ip, port, dn)


if __name__ == "__main__":
    name = sys.argv[1] if len(sys.argv) > 1 else ""
    ip = sys.argv[2] if len(sys.argv) > 2 else ""
    port = int(sys.argv[3]) if len(sys.argv) > 3 else 0
    try:
        sys.exit(asyncio.run(main(name, ip, port)))
    except Exception as exc:
        print(f"ERROR: {type(exc).__name__}: {exc}", flush=True)
        sys.exit(1)
