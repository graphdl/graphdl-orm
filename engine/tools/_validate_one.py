"""Worker: compile+validate ONE app, print 'VIOL <n> <fts>' | 'CLEAN' | 'ERR <msg>'."""
import os, sys, importlib.util, shutil, tempfile
_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
APPS = os.path.join(os.path.dirname(_ROOT), "..", "apps")
name = sys.argv[1]
spec = importlib.util.spec_from_file_location("pyarest", os.path.join(_ROOT, "python", "__init__.py"),
    submodule_search_locations=[os.path.join(_ROOT, "python")])
m = importlib.util.module_from_spec(spec); sys.modules["pyarest"] = m; spec.loader.exec_module(m)
import pyarest.prims  # noqa
from pyarest import apps as A
base = A.default_base()
scratch = tempfile.mkdtemp(prefix=f"vo_{name}_")
try:
    rd = os.path.join(scratch, name, "readings"); os.makedirs(rd)
    for f in os.listdir(os.path.join(APPS, name, "readings")):
        if f.endswith(".md"): shutil.copy(os.path.join(APPS, name, "readings", f), os.path.join(rd, f))
    A.Registry(scratch, base_dir=base).compile(name)
    v = A.Registry(scratch, base_dir=base).validate(name)
    viols = v.get("violations", [])
    if viols:
        print(f"VIOL {len(viols)} {[x['fact_type'] for x in viols][:4]}")
    else:
        print("CLEAN")
except Exception as e:
    print(f"ERR {str(e)[:70]}")
finally:
    shutil.rmtree(scratch, ignore_errors=True)
