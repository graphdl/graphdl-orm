"""Native-vs-Python byte-parity over a fixed set of CLEAN, inline CONSTRUCT models (no-base).

SCOPE / WARNING: the CORPUS below is hand-written single-line-per-statement models with NO
markdown headers or comment lines, so the `text.replace("\\n", " ")` composition in native()
is safe HERE. Do NOT reuse this pattern on real app readings/*.md — inlining a markdown '#'
header into a statement corrupts it and manufactures a phantom divergence (bit us 2026-07-13).
For REAL apps, use `apps_compile_parity.py`, which drives both hosts through the real
apps_compile/Registry.compile flow (readings read from disk, newlines preserved).
"""
import importlib.util, os, sys, json, subprocess
_ROOT = r"C:/Users/lippe/Repos/arest/engine"
spec = importlib.util.spec_from_file_location("pyarest", os.path.join(_ROOT,"python","__init__.py"),
    submodule_search_locations=[os.path.join(_ROOT,"python")])
mod = importlib.util.module_from_spec(spec); sys.modules["pyarest"]=mod; spec.loader.exec_module(mod)
import pyarest.prims  # noqa
from pyarest import apps
TMP=r"C:/Users/lippe/.claude/jobs/2b70be63/tmp"; G=os.path.join(_ROOT,"shared","forml2-grammar.store.json")
BIN=os.path.join(_ROOT,"rust","target","release","arest.exe")
CORPUS={
 "subtype_mandatory":"Employee is an entity type.\nManager is an entity type.\nManager is a kind of Employee.\nName is a value type.\nEmployee has Name.\nEach Employee has some Name.\n",
 "mn_fact":"Student is an entity type.\nCourse is an entity type.\nStudent enrolls in Course.\n",
 "derivation_iff":"Person is an entity type.\nPerson is an adult iff Person has an Age of at least 18.\n",
 "ring":"Person is an entity type.\nPerson is a parent of Person.\n",
 "value_range":"Person is an entity type.\nAge is a value type.\nPerson has Age.\nAge is between 0 and 120.\n",
}
def cellmap(store):
    m={}
    for c in store.get("d",[]):
        if isinstance(c,list) and len(c)>=3 and c[0]=="CELL": m[c[1]]=json.dumps(c[2],sort_keys=True)
    return m
def native(text):
    OUT=os.path.join(TMP,"c_nat.store.json")
    if os.path.exists(OUT): os.remove(OUT)
    req={"op":"compile_model","text":text.replace("\n"," ").strip(),"grammar_sidecar":G,"save_path":OUT}
    r=subprocess.run([BIN,"--serve"],input=json.dumps(req)+"\n",capture_output=True,text=True,timeout=90)
    rep=json.loads([l for l in r.stdout.splitlines() if l.strip()][0]).get("result",{})
    return json.load(open(OUT,encoding="utf-8")), rep
for name,text in CORPUS.items():
    root=os.path.join(TMP,"corp",name); rd=os.path.join(root,"a","readings"); os.makedirs(rd,exist_ok=True)
    open(os.path.join(rd,"core.md"),"w",encoding="utf-8").write(text)
    try: apps.Registry(root).compile("a"); py=json.load(open(os.path.join(root,"a","a.store.json"),encoding="utf-8"))
    except Exception as e: print(f"{name}: PY compile ERR {e}"); continue
    try: nat,rep=native(text)
    except Exception as e: print(f"{name}: NATIVE ERR {e}"); continue
    pm,nm=cellmap(py),cellmap(nat); pk,nk=set(pm),set(nm)
    diff=[k for k in pk&nk if pm[k]!=nm[k]]
    ok = not (pk^nk) and not diff
    print(f"{name:20} py={len(pk):2} nat={len(nk):2} classified={rep.get('classified')}/{rep.get('total')} unparsed={len(rep.get('unparsed',[]))} :: {'PARITY' if ok else 'DIVERGE only_py='+str(sorted(pk-nk))+' only_nat='+str(sorted(nk-pk))+' diff='+str(diff)}")
