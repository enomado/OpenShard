#!/usr/bin/env python3
"""Read a five-strip shadow dump and answer questions about it in numbers.

The dump is what `tests/traced.rs` and `examples/boxes.rs` write when
`OPENSHARD_TRACED_DUMP` / `OPENSHARD_FRAME_DUMP` name a directory —
`oracle::pathtrace::Verdict::strips`, in this order:

    0 the frame's own lit/shadow decision      3 why an uncompared pixel was not compared
    1 the path tracer's                        4 which solid the frame drew, one colour a body
    2 where the two differ

**Why a tool and not an eye.** Every wrong reading this track has produced came
from looking at these pictures instead of measuring them: a stale dump read as a
lighting fault, a mask laid over a frame from the other tool and so placed one
tile east, a composite sliced by `width // 3` and read as a three-pixel camera
offset. Each of those is a question with a numeric answer, and each is below.

    ./tools/mask_probe.py overlay DUMP.png OUT.png [X0 Y0 X1 Y1]
        The frame's shadow drawn *on* the body map: a body's own colour, pale
        where the flame reaches it and dark where it does not. This is the
        picture that makes a shadow boundary placeable — "which solid is this
        edge on" is a question the two black-and-white masks cannot answer.

    ./tools/mask_probe.py probe DUMP.png X Y [RADIUS]
        The neighbourhood of one pixel as text: which body drew it, and what the
        two renderers each say about the light there. `!` and `?` mark the pixels
        they disagree on, so "is this edge ours or is it the geometry" is read
        off the map rather than argued.

    ./tools/mask_probe.py seams DUMP.png [LIT.png]
        Every pixel where two *different* solids of the same tread level meet
        side by side and the shadow decision flips across the join — the seam
        census in its cheapest form. With a lit frame of the same camera, it also
        reports how large the step actually is in the shaded picture, which is
        the number that says whether a seam is visible to a player or only to a
        visibility mask.

**One camera.** Every strip of a dump comes from one frame, so overlaying them is
exact by construction. Overlaying anything from a *different* run is not: the two
tools frame a scene differently — `examples/boxes.rs` centres on the scene's own
tile bounds, `tests/traced.rs` on a named tile — and for a run of three flights
those differ by a whole tile.

Needs Pillow, and nothing else.
"""

import sys

from PIL import Image

# `png::write_strips`' own ruler. A composite of `n` panels is
# `n * SIDE + (n - 1) * RULE` wide, so slicing it by `width // n` is off by a
# pixel per ruler — which is exactly how a phantom camera offset gets measured.
RULE = 2

# `oracle::pathtrace::BODY_COLOURS`, and the two must stay one list: this file
# names a body by matching its colour exactly, so a palette that drifted would
# silently start reporting every body as "not a body".
BODY_COLOURS = [
    (222, 60, 60),
    (60, 200, 90),
    (70, 120, 240),
    (230, 180, 40),
    (200, 70, 210),
    (40, 200, 210),
    (240, 130, 50),
    (140, 220, 60),
    (110, 90, 230),
]
GROUND = (32, 32, 40)


def strips(path):
    """The five panels of a dump, as images, with the rulers dropped."""
    image = Image.open(path).convert("RGB")
    width, side = image.size
    count = (width + RULE) // (side + RULE)
    if count * side + (count - 1) * RULE != width:
        raise SystemExit(f"{path} is {width}x{side}: not a strip composite of square panels")
    return [image.crop((k * (side + RULE), 0, k * (side + RULE) + side, side)) for k in range(count)]


def body_of(scene, x, y):
    """Which solid the frame drew at a pixel: an index, `'g'` for the ground,
    `'.'` for nothing at all."""
    colour = scene[x, y]
    if colour == GROUND:
        return "g"
    if colour == (0, 0, 0):
        return "."
    try:
        return BODY_COLOURS.index(colour)
    except ValueError:
        return "?"


def overlay(argv):
    panels = strips(argv[0])
    engine, scene = panels[0].load(), panels[4]
    out = scene.copy()
    source, target = scene.load(), out.load()
    side = out.width
    for y in range(side):
        for x in range(side):
            colour = source[x, y]
            if colour == (0, 0, 0):
                continue
            if engine[x, y][0] > 128:
                target[x, y] = tuple(min(255, channel + 150) for channel in colour)
            else:
                target[x, y] = tuple(channel // 4 for channel in colour)
    if len(argv) > 2:
        box = tuple(int(v) for v in argv[2:6])
        out = out.crop(box)
    out = out.resize((out.width * 2, out.height * 2), Image.NEAREST)
    out.save(argv[1])
    print(f"wrote {argv[1]} {out.size}")


def probe(argv):
    panels = strips(argv[0])
    engine, traced, scene = panels[0].load(), panels[1].load(), panels[4].load()
    at_x, at_y = int(argv[1]), int(argv[2])
    radius = int(argv[3]) if len(argv) > 3 else 12
    side = panels[0].width
    print(f"around ({at_x}, {at_y}) — body per pixel, then the frame's own bit (# lit, _ dark)")
    print(f"  ! the frame lit a pixel the tracer shadowed, ? the other way round")
    for y in range(max(0, at_y - radius), min(side, at_y + radius + 1)):
        bodies, bits = "", ""
        for x in range(max(0, at_x - radius), min(side, at_x + radius + 1)):
            lit, saw = engine[x, y][0] > 128, traced[x, y][0] > 128
            bodies += "!" if lit and not saw else "?" if saw and not lit else str(body_of(scene, x, y))
            bits += "#" if lit else "_"
        print(f"  {bodies}")
        print(f"  {bits}")


def seams(argv):
    panels = strips(argv[0])
    engine, scene = panels[0].load(), panels[4].load()
    shaded = Image.open(argv[1]).convert("RGB").load() if len(argv) > 1 else None
    side = panels[0].width
    found = {}
    for y in range(side):
        for x in range(side - 1):
            here, next_along = body_of(scene, x, y), body_of(scene, x + 1, y)
            if not isinstance(here, int) or not isinstance(next_along, int) or here == next_along:
                continue
            # Same tread of two different flights: the join `docs/occluders.md`
            # is about, and the only one where a step is a claim about seams
            # rather than about a corner the geometry really has.
            if here % 3 != next_along % 3:
                continue
            if (engine[x, y][0] > 128) == (engine[x + 1, y][0] > 128):
                continue
            step = 0
            if shaded is not None:
                step = abs(max(shaded[x, y]) - max(shaded[x + 1, y]))
            found.setdefault((here, next_along), []).append(step)
    if not found:
        print("no seam of two flights has the shadow decision flip across it")
        return
    for pair, steps in sorted(found.items()):
        line = f"bodies {pair[0]}|{pair[1]}: {len(steps)} pixels where the shadow flips across the seam"
        if shaded is not None:
            line += f"; in the shaded frame the step is at most {max(steps)}, mean {sum(steps) / len(steps):.1f} of 255"
        print(line)


COMMANDS = {"overlay": overlay, "probe": probe, "seams": seams}

if __name__ == "__main__":
    if len(sys.argv) < 3 or sys.argv[1] not in COMMANDS:
        raise SystemExit(__doc__)
    COMMANDS[sys.argv[1]](sys.argv[2:])
