Upstream: Bevy 0.19.1 `bevy_gilrs`, copied from the installed Cargo source.
Original MIT/Apache licenses and Cargo provenance are preserved.

The weak rumble motor now uses Gilrs' Weak effect instead of Strong and
explicitly uses the same requested Replay duration as the strong channel.
Gilrs' default one-tick Replay repeats; the outer Repeat::For governs total
duration, so this alignment is not claimed as a previously truncated pulse.
`ConstantRumble` adapts the original game's persistent motor-value output:
two infinite, independently gained FF effects are created once per active
controller ownership period and reused when amplitudes change. There is no
per-frame effect creation or overlapping impulse queue. None, disconnect and
backend owner drop release those RAII handles. This is a host output bridge,
not an effect timer or gameplay mixer. The normal Add/Stop API is preserved.
