#!/usr/bin/env python3
"""Regenerate twinkle.mid, the byte fixture for the MIDI binary-parsing test.

The bytes are committed so the test needs no toolchain, but this script keeps
them auditable. Run it from anywhere; it writes next to itself.

    python3 make_fixture.py

Produces a spec-correct Standard MIDI File (format 0, one track):
    MThd len=6  format=0  ntracks=1  division=96
    MTrk:
        delta 0   NoteOn  ch0 note60 vel64   (90 3C 40)
        delta 96  NoteOff ch0 note60 vel64   (80 3C 40)
        delta 0   Meta SetTempo 500000 us    (FF 51 03 07 A1 20)
        delta 0   Meta EndOfTrack            (FF 2F 00)

`file(1)` reports: "Standard MIDI data (format 0) using 1 track at 1/96".
"""
import os
import struct


def vlq(value: int) -> bytes:
    """Encode an int as a MIDI variable-length quantity (MSB group first)."""
    out = [value & 0x7F]
    value >>= 7
    while value:
        out.insert(0, (value & 0x7F) | 0x80)
        value >>= 7
    return bytes(out)


def build() -> bytes:
    track = b""
    track += vlq(0) + bytes([0x90, 60, 64])  # NoteOn  ch0
    track += vlq(96) + bytes([0x80, 60, 64])  # NoteOff ch0
    # Set Tempo 500000 us/quarter-note (the 3-byte payload after FF 51 03).
    track += vlq(0) + bytes([0xFF, 0x51, 0x03]) + struct.pack(">I", 500000)[1:]
    track += vlq(0) + bytes([0xFF, 0x2F, 0x00])  # EndOfTrack

    mthd = b"MThd" + struct.pack(">IHHH", 6, 0, 1, 96)
    mtrk = b"MTrk" + struct.pack(">I", len(track)) + track
    return mthd + mtrk


def main() -> None:
    data = build()
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "twinkle.mid")
    with open(path, "wb") as handle:
        handle.write(data)
    print(f"wrote {len(data)} bytes to {path}")
    print("hex:", data.hex())


if __name__ == "__main__":
    main()
