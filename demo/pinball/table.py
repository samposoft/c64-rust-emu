"""STARFALL table geometry.

Everything is in screen pixels (320x200, y downwards). The solid parts of
the table are analytic shapes with a signed distance function (negative
inside), so the art, the collision maps and the checks all come from the
same numbers.

    materials: WALL (plain walls), RUBBER (posts and rubbers, bouncier),
    and the specials the game has to recognise (bumpers, sling faces).
"""

import math

import numpy as np

# ---------------------------------------------------------------- constants

BALL_R = 3.5              # ball radius (the sprite is a 7-pixel disc)

# playfield: left wall face at x 16, lane divider 181-183, right wall face 196
PF_L = 16
PF_R = 196
ARCH_CX, ARCH_CY, ARCH_R = 106, 96, 90   # inner face of the top arch
MIDX = 98.5               # mirror axis of the playfield (16..181)

LANE_X0, LANE_X1 = 181, 183              # plunger lane divider
LANE_TOP = 112
LANE_CX = 190                            # ball centre in the plunger lane
PLUNGER_Y = 188                          # ball centre resting on the plunger

GATE_X = 182                             # one-way gate at the lane exit
GATE_Y0, GATE_Y1 = 40, 112

# flippers: pivot, length (pivot to tip centre), radii, angles (256 units,
# positive = clockwise = down for the left flipper)
FLIP_L = 26
FLIP_R1 = 4.0
FLIP_R2 = 2.5
FLIP_REST = 24
FLIP_UP = -16
LFLIP = (70.0, 176.0)
RFLIP = (2 * MIDX - LFLIP[0], LFLIP[1])

# materials
M_WALL, M_RUBBER, M_SPECIAL = 1, 2, 3

# special objects (material 3); index = object id in the game
BUMPERS = [(84, 62, 8), (124, 62, 8), (104, 84, 8)]

# drop targets: bank on the left wall, face at x = TARGET_X
TARGET_X = 22
TARGET_Y = [100, 108, 116, 124]          # edges: 3 targets of 8 pixels

SAUCER = (156, 98)                       # kickout hole centre

# upper-left block: its diagonal face carries the S-T-A-R stand-up targets
BLOCK_TOP = (71, 40)
BLOCK_BOT = (16, 95)
STAR_T = [0.22, 0.40, 0.58, 0.76]        # target centres along the face
STAR_W = 8                               # target width along the face


def mirror_pts(pts):
    return [(2 * MIDX - x, y) for x, y in pts]


# ---------------------------------------------------------------- shapes


class Shape:
    def __init__(self, kind, mat, obj, *args):
        self.kind, self.mat, self.obj, self.args = kind, mat, obj, args

    def sdf(self, x, y):
        a = self.args
        if self.kind == "circle":
            cx, cy, r = a
            return np.hypot(x - cx, y - cy) - r
        if self.kind == "capsule":
            x0, y0, x1, y1, r = a
            vx, vy = x1 - x0, y1 - y0
            t = np.clip(((x - x0) * vx + (y - y0) * vy) / (vx * vx + vy * vy), 0, 1)
            return np.hypot(x - x0 - t * vx, y - y0 - t * vy) - r
        if self.kind == "poly":
            return poly_sdf(a[0], x, y) - a[1]
        if self.kind == "outside":
            # everything outside the playfield (arch + rectangle below it)
            d_disc = np.hypot(x - ARCH_CX, y - ARCH_CY) - ARCH_R
            dx = np.maximum(PF_L - x, x - PF_R)
            dy = np.maximum(ARCH_CY - y, y - 400)
            d_rect = np.where((dx > 0) | (dy > 0),
                              np.hypot(np.maximum(dx, 0), np.maximum(dy, 0)),
                              np.maximum(dx, dy))
            return -np.minimum(d_disc, d_rect)
        raise ValueError(self.kind)


def poly_sdf(pts, x, y):
    """Signed distance to a simple polygon (negative inside)."""
    pts = [np.array(p, float) for p in pts]
    d = np.full(np.shape(x), 1e9)
    s = np.ones(np.shape(x))
    n = len(pts)
    for i in range(n):
        a, b = pts[i], pts[(i + 1) % n]
        e = b - a
        wx, wy = x - a[0], y - a[1]
        t = np.clip((wx * e[0] + wy * e[1]) / (e @ e), 0, 1)
        bx, by = wx - e[0] * t, wy - e[1] * t
        d = np.minimum(d, bx * bx + by * by)
        c1 = y >= a[1]
        c2 = y < b[1]
        c3 = e[0] * wy > e[1] * wx
        flip = (c1 & c2 & c3) | (~c1 & ~c2 & ~c3)
        s = np.where(flip, -s, s)
    return s * np.sqrt(d)


def build_shapes():
    S = []
    W, R, X = M_WALL, M_RUBBER, M_SPECIAL
    S.append(Shape("outside", W, None))
    # plunger lane divider, rounded top
    S.append(Shape("capsule", W, None, 182, LANE_TOP, 182, 260, 1.5))
    # upper-left block (its top side is hidden in the arch)
    S.append(Shape("poly", W, None,
                   [(BLOCK_TOP[0], 0), BLOCK_TOP, BLOCK_BOT, (0, BLOCK_BOT[1]), (0, 0)], 1.5))
    # S-T-A-R stand-up targets, 2 pixels in front of the face
    fx, fy = BLOCK_BOT[0] - BLOCK_TOP[0], BLOCK_BOT[1] - BLOCK_TOP[1]
    fl = math.hypot(fx, fy)
    ux, uy = fx / fl, fy / fl
    nx, ny = uy, -ux                     # face normal (down-right)
    for i, t in enumerate(STAR_T):
        cx = BLOCK_TOP[0] + fx * t + nx * 1.5
        cy = BLOCK_TOP[1] + fy * t + ny * 1.5
        h = STAR_W / 2 - 1.5
        S.append(Shape("capsule", X, 5 + i, cx - ux * h, cy - uy * h, cx + ux * h, cy + uy * h, 1.5))
    # guide under the lane gate: balls falling along the right wall go back
    # into play instead of straight into the right outlane
    S.append(Shape("capsule", R, None, 182, LANE_TOP, 173, 123, 1.5))
    # top lane guides
    for gx in (94, 114, 134):
        S.append(Shape("capsule", R, None, gx, 24, gx, 40, 1.5))
    # pop bumpers
    for i, (cx, cy, r) in enumerate(BUMPERS):
        S.append(Shape("circle", X, i, cx, cy, r))
    # drop target bank: recess posts above and below (targets are dynamic)
    S.append(Shape("circle", R, None, 18.5, TARGET_Y[0] - 3.5, 3.5))
    S.append(Shape("circle", R, None, 18.5, TARGET_Y[-1] + 3.5, 3.5))
    for side in (0, 1):
        m = (lambda p: p) if side == 0 else mirror_pts
        # outlane / inlane divider: post, straight down, bend to the flipper
        top, bend, end = (29, 134), (29, 156), (63, 173)
        p = m([top, bend, end])
        S.append(Shape("capsule", R, None, *p[0], *p[1], 1.8))
        S.append(Shape("capsule", W, None, *p[1], *p[2], 1.8))
        # slingshot: triangle with rounded corners; the long face kicks
        tri = m([(44, 128), (44, 150), (57, 160)])
        S.append(Shape("poly", W, None, tri, 2.0))
        kick = m([(46.5, 127), (59.5, 159)])
        S.append(Shape("capsule", X, 3 + side, *kick[0], *kick[1], 1.5))
        # flipper pivot base (the flipper itself is dynamic)
    # saucer rim: a ring open towards the bottom-left, drawn as two arcs
    return S


SHAPES = build_shapes()


def sdf_all(x, y):
    """Distance to the nearest solid and the index of that shape."""
    ds = np.stack([s.sdf(x, y) for s in SHAPES])
    idx = np.argmin(ds, axis=0)
    return np.take_along_axis(ds, idx[None], 0)[0], idx


def normal_at(x, y, h=0.05):
    dx = sdf_all(x + h, y)[0] - sdf_all(x - h, y)[0]
    dy = sdf_all(x, y + h)[0] - sdf_all(x, y - h)[0]
    n = np.hypot(dx, dy) + 1e-12
    return dx / n, dy / n


# ---------------------------------------------------------------- flippers


def flipper_poly(ang256, mirror=False):
    """Signed distance function of a flipper at the given angle."""
    a = ang256 * 2 * math.pi / 256
    px, py = RFLIP if mirror else LFLIP
    ux, uy = math.cos(a), math.sin(a)
    if mirror:
        ux = -ux

    def f(x, y):
        dx, dy = x - px, y - py
        t = np.clip((dx * ux + dy * uy) / FLIP_L, 0, 1)
        r = FLIP_R1 + (FLIP_R2 - FLIP_R1) * t
        return np.hypot(dx - t * FLIP_L * ux, dy - t * FLIP_L * uy) - r
    return f
