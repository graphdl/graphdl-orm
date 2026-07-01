"""DEFS — the definition cell (Backus §13.3.5) and the enumerable boundary."""
from dataclasses import dataclass
from typing import Callable, Optional


@dataclass(frozen=True)
class Def:
    key: object          # the atom value: str or int
    origin: str          # "registered" | "compiled"
    impl: object         # registered → Python callable; compiled → an Object


class Defs:
    """The DEFS cell.

    registered = host-supplied callable (the enumerable boundary);
    compiled   = an Object o, whose meaning is ρ(o).
    """

    def __init__(self):
        self._t: dict = {}

    def register(self, key, fn: Callable):
        self._t[key] = Def(key, "registered", fn)

    def define(self, key, obj):
        self._t[key] = Def(key, "compiled", obj)

    def get(self, key) -> Optional[Def]:
        return self._t.get(key)

    def boundary(self):
        """Cor. Enumerable boundary: the registered definitions."""
        return [d for d in self._t.values() if d.origin == "registered"]


DEFS = Defs()   # process-global seed store
