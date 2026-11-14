import sys, importlib, pkgutil, os
sys.path.insert(0, os.path.dirname(__file__))
from lib import D
fns = {}
for m in sorted(f[:-3] for f in os.listdir(os.path.dirname(__file__)) if f.startswith("d_") and f.endswith(".py")):
    mod = importlib.import_module(m)
    for n in dir(mod):
        f = getattr(mod, n)
        if callable(f) and getattr(f, "__module__", "") == m and not n.startswith("_"):
            fns[n.replace("_", "-")] = f
alvo = sys.argv[1:] or list(fns)
for n in alvo:
    print("gerando", n, fns[n]().salvar())
