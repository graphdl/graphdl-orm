"""Repo layout, resolved once: the polyglot monorepo is shared/ (canonical
cross-host sources: the grammar and module readings any carrier ingests), python/
(this host), rust/ (the Rust host). Hosts locate the shared sources and each other
through the repo root, never through their own package position."""
import os

_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def root():
    return _ROOT


def shared(name):
    """A canonical shared source file (readings any host ingests)."""
    return os.path.join(_ROOT, "shared", name)


def rust_bin(name):
    return os.path.join(_ROOT, "rust", "target", "release",
                        name + (".exe" if os.name == "nt" else ""))
