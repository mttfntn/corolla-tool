"""Draws the app icon (the wheel's copyright-style hub) to the given PNG path, standard library only."""
import math
import struct
import sys
import zlib

SIZE = 1024
SAMPLES = 3
BLACK = (0, 0, 0, 255)
WHITE = (255, 255, 255, 255)


def coverage(x, y):
    dx, dy = x - SIZE / 2, y - SIZE / 2
    radius = math.hypot(dx, dy)
    if radius > 500:
        return None
    # The thin white edge keeps the black disc visible on a dark taskbar.
    if radius > 476:
        return WHITE
    if 330 < radius < 400:
        return WHITE
    if 135 < radius < 225:
        # The C's opening faces right.
        angle = abs(math.atan2(dy, dx))
        if angle > math.radians(48):
            return WHITE
    return BLACK


def pixel(px, py):
    total = [0, 0, 0, 0]
    for sx in range(SAMPLES):
        for sy in range(SAMPLES):
            colour = coverage(px + (sx + 0.5) / SAMPLES, py + (sy + 0.5) / SAMPLES)
            if colour:
                alpha = colour[3]
                for i in range(3):
                    total[i] += colour[i] * alpha
                total[3] += alpha
    if total[3] == 0:
        return b"\0\0\0\0"
    n = SAMPLES * SAMPLES
    return bytes((total[0] // total[3], total[1] // total[3], total[2] // total[3], total[3] // n))


def chunk(kind, data):
    body = kind + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))


rows = b"".join(b"\0" + b"".join(pixel(x, y) for x in range(SIZE)) for y in range(SIZE))
with open(sys.argv[1], "wb") as out:
    out.write(b"\x89PNG\r\n\x1a\n")
    out.write(chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0)))
    out.write(chunk(b"IDAT", zlib.compress(rows, 6)))
    out.write(chunk(b"IEND", b""))
