"""Shared dimensions (metres, glTF space). The seated-kid clips and the kit are
both built from these, so the contract's seated geometry holds by
construction: kid origin on the `Chair` origin, `Workstation` at z = +WS_Z."""

SEAT_Y = 0.42        # chair seat top (contract: ≈0.42)
WS_Z = 0.55          # workstation offset from the chair (contract)
DESK_Y = 0.60        # desk top surface
KB_Z = -0.20         # keyboard centre, workstation-local (toward the kid)
KB_Y = DESK_Y + 0.018  # keyboard key-top height
KID_HEIGHT = 1.10    # standing height (contract: ≈1.0–1.1)
ROBOT_HEIGHT = 1.6   # headmaster standing height
