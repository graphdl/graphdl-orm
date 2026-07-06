"""Root conftest: register the `pyarest` package from the python/ host directory.

The Python host's modules live directly in python/ (the polyglot layout: shared/
holds only polyglot sources, python/ and rust/ are the hosts), and that directory is
not named `pyarest`, so this file constructs the package explicitly. Installed use
gets the same shape from pyproject's package-dir mapping."""
import importlib.util
import os
import sys

_ROOT = os.path.dirname(os.path.abspath(__file__))

if "pyarest" not in sys.modules:
    spec = importlib.util.spec_from_file_location(
        "pyarest", os.path.join(_ROOT, "python", "__init__.py"),
        submodule_search_locations=[os.path.join(_ROOT, "python")])
    mod = importlib.util.module_from_spec(spec)
    sys.modules["pyarest"] = mod
    spec.loader.exec_module(mod)
