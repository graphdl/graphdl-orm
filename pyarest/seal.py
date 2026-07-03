"""Field-level encryption per the platform arc: sensitivity DERIVES from DATA TYPES (a
value type whose conceptual data type is in the sensitive set seals by default), the
MODE derives from CONSTRAINTS (a sealed role inside a uniqueness span or a reference
scheme must seal deterministically, because equality has to survive sealing — NATEQ on
ciphertexts; everything else seals randomized), and the KEY SCOPE is the tenant. The
cipher itself is a boundary def: the one here is TEST-GRADE (an HMAC-SHA256 keystream,
clearly not production cryptography — production binds real AEAD, as the old engine's
cell_aead did); the ENGINE's part is the derivation and the interface, which is what
the tests pin."""
import base64
import hashlib
import hmac
import json
import os

from . import system


SENSITIVE_DATA_TYPES = {"SensitiveText", "Secret", "PII"}
_MARK = "enc1:"


def _data_types(D):
    out = {}
    for r in system._pop_rows(D, "data_type"):
        text = r[0] if r else ""
        if " is " in text:
            name, dt = text.split(" is ", 1)
            out[name.strip()] = dt.strip()
    return out


def plan(D):
    """The derivation: which ⟨fact type, column⟩ seals, in which mode, plus which nouns'
    IDENTIFIERS seal (reference modes on sensitive value types — always deterministic,
    identifiers ARE equality)."""
    dts = _data_types(D)
    sensitive = {vt for vt, dt in dts.items() if dt in SENSITIVE_DATA_TYPES}
    spans = {}
    for r in system._pop_rows(D, "spans"):
        if len(r) == 2:
            spans.setdefault(r[0], set()).add(r[1])
    uc_pos = {}
    for c in system._pop_rows(D, "constraint"):
        if len(c) >= 3 and c[1] in ("uniqueness", "spanning_uniqueness"):
            uc_pos.setdefault(c[2], set()).update(spans.get(c[0], set()))
    roles = {}
    for r in system._pop_rows(D, "role"):
        if len(r) >= 4 and r[3] in sensitive:
            ft, pos = r[1], r[2]
            det = pos in uc_pos.get(ft, set())
            roles[(ft, pos)] = "deterministic" if det else "randomized"
    ids = {}
    for r in system._pop_rows(D, "refMode"):
        if len(r) >= 2 and r[1] in sensitive:
            ids[r[0]] = "deterministic"
    return {"roles": roles, "ids": ids}


# ============================ the cipher boundary (TEST-GRADE) ================
def _stream(key, nonce, n):
    out = b""
    counter = 0
    while len(out) < n:
        out += hmac.new(key, nonce + counter.to_bytes(4, "big"), hashlib.sha256).digest()
        counter += 1
    return out[:n]


def seal(key, value, deterministic=False):
    """TEST-GRADE sealing: deterministic mode derives the nonce from the plaintext (same
    value, same ciphertext — equality survives), randomized mode draws it fresh."""
    data = json.dumps(value, ensure_ascii=False).encode("utf-8")
    nonce = (hmac.new(key, data, hashlib.sha256).digest()[:8] if deterministic
             else os.urandom(8))
    ct = bytes(a ^ b for a, b in zip(data, _stream(key, nonce, len(data))))
    return _MARK + base64.b64encode(nonce + ct).decode("ascii")


def unseal(key, token):
    if not (isinstance(token, str) and token.startswith(_MARK)):
        return token
    raw = base64.b64decode(token[len(_MARK):])
    nonce, ct = raw[:8], raw[8:]
    data = bytes(a ^ b for a, b in zip(ct, _stream(key, nonce, len(ct))))
    return json.loads(data.decode("utf-8"))


def seal_rows(key, rows, cols_modes):
    out = []
    for row in rows:
        row = list(row)
        for (pos, mode) in cols_modes:
            if pos - 1 < len(row):
                row[pos - 1] = seal(key, row[pos - 1], mode == "deterministic")
        out.append(tuple(row))
    return tuple(out)


def unseal_rows(key, rows):
    return tuple(tuple(unseal(key, v) if isinstance(v, str) else v for v in row)
                 for row in rows)
