// The FFP reducer, mirroring python/delta.py (the semantics of record; both
// held equal by the cross-host differential). Values: string/long/double/bool
// atoms, object[] sequences, Bot the bottom sentinel, AppTag heading an
// application node. Metacomposition is the only mechanism: a sequence applied
// as operator dispatches on its head; a string atom resolves through the def
// store; an integer atom is a selector. Comparators and arithmetic coerce
// int-first, then float (the claude analytics family's lesson), and equality
// stays NATEQ (same type and equal, so 1 never equals 1.0).
namespace Arestlam;

sealed class Bot
{
    public static readonly Bot Value = new();
    private Bot() { }
    public override string ToString() => "⊥";
}

sealed class AppTag
{
    public static readonly AppTag Value = new();
    private AppTag() { }
}

static class Reducer
{
    static readonly object T = "T";
    static readonly object F = "F";

    internal static Dictionary<string, object> Store = new();

    internal static void LoadCanon()
    {
        Store.Clear();
        foreach (var kv in Canon.LoadAll())
            Store[kv.Key] = kv.Value;
    }

    static bool IsSeq(object o) => o is object[];

    static object MkSeq(IEnumerable<object> items)
    {
        var t = items.ToArray();
        return t.Any(i => ReferenceEquals(i, Bot.Value)) ? Bot.Value : t;
    }

    internal static object App(object f, object x) =>
        new object[] { AppTag.Value, f, x };

    static bool EqObj(object a, object b)
    {
        if (a is object[] sa && b is object[] sb)
            return sa.Length == sb.Length &&
                   sa.Zip(sb).All(p => EqObj(p.First, p.Second));
        if (IsSeq(a) || IsSeq(b)) return false;
        return a.GetType() == b.GetType() && a.Equals(b);
    }

    static double? ToNum(object x) => x switch
    {
        bool => null,
        long i => i,
        double d => d,
        string s when long.TryParse(s.Trim(), out var i) => i,
        string s when double.TryParse(s.Trim(), out var d) => d,
        _ => null,
    };

    static object Arith(object a, object b, Func<long, long, long> fi,
                        Func<double, double, double> fd)
    {
        if (a is not string && a is not long && a is not double) return Bot.Value;
        if (b is not string && b is not long && b is not double) return Bot.Value;
        long? ia = a is long la ? la : (a is string sa && long.TryParse(sa.Trim(), out var pa) ? pa : null);
        long? ib = b is long lb ? lb : (b is string sb && long.TryParse(sb.Trim(), out var pb) ? pb : null);
        if (ia is long xa && ib is long xb) return fi(xa, xb);
        var na = ToNum(a); var nb = ToNum(b);
        if (na is double da && nb is double db) return fd(da, db);
        return Bot.Value;
    }

    static object Cmp(object a, object b, Func<double, double, bool> rel,
                      Func<string, string, bool> rels)
    {
        if (IsSeq(a) || IsSeq(b)) return Bot.Value;
        var na = ToNum(a); var nb = ToNum(b);
        if (na is double da && nb is double db) return rel(da, db) ? T : F;
        if (a is string x && b is string y) return rels(x, y) ? T : F;
        if (a.GetType() == b.GetType() && a is bool ba && b is bool bb)
            return rel(ba ? 1 : 0, bb ? 1 : 0) ? T : F;
        return Bot.Value;
    }

    internal static object Mu(object e)
    {
        while (true)
        {
            if (e is object[] app && app.Length == 3 &&
                ReferenceEquals(app[0], AppTag.Value))
            {
                var f = Mu(app[1]);
                var x = Mu(app[2]);
                if (ReferenceEquals(f, Bot.Value) ||
                    ReferenceEquals(x, Bot.Value)) return Bot.Value;
                if (f is object[] seq)
                {
                    if (seq.Length == 0) return Bot.Value;
                    e = App(seq[0], new object[] { seq, x });   // metacomposition
                    continue;
                }
                if (f is long sel)
                {
                    if (x is object[] row && row.Length >= sel && sel >= 1)
                        return row[sel - 1];
                    return Bot.Value;
                }
                if (f is string name)
                {
                    var step = Prim(name, x, out var next, out var handled);
                    if (handled) { if (next) { e = step; continue; } return step; }
                    if (Store.TryGetValue(name, out var impl))
                    { e = App(impl, x); continue; }
                    return Bot.Value;
                }
                return Bot.Value;
            }
            return e;
        }
    }

    // Controlling forms + prims. `next` means the answer is an expression for
    // the trampoline; otherwise it is a value. `handled` false falls to defs.
    static object Prim(string name, object x, out bool next, out bool handled)
    {
        next = false; handled = true;
        object[] Pair() => x as object[];
        switch (name)
        {
            case "COMP":
            {
                var o = Pair(); var whole = (object[])o[0]; var arg = o[1];
                var acc = arg;
                for (int i = whole.Length - 1; i >= 1; i--)
                    acc = App(whole[i], acc);
                next = true; return acc;
            }
            case "CONS":
            {
                var o = Pair(); var whole = (object[])o[0]; var arg = o[1];
                return MkSeq(whole.Skip(1).Select(f => Mu(App(f, arg))));
            }
            case "CONST":
            {
                var o = Pair(); var whole = (object[])o[0];
                return whole.Length >= 2 ? whole[1] : Bot.Value;
            }
            case "COND":
            {
                var o = Pair(); var whole = (object[])o[0]; var arg = o[1];
                if (whole.Length < 4) return Bot.Value;
                var pv = Mu(App(whole[1], arg));
                if (Equals(pv, T)) { next = true; return App(whole[2], arg); }
                if (Equals(pv, F)) { next = true; return App(whole[3], arg); }
                return Bot.Value;
            }
            case "ALPHA":
            {
                var o = Pair(); var whole = (object[])o[0]; var arg = o[1];
                if (whole.Length < 2) return Bot.Value;
                if (arg is object[] xs)
                    return xs.Length == 0 ? Array.Empty<object>()
                         : MkSeq(xs.Select(xi => Mu(App(whole[1], xi))));
                return Bot.Value;
            }
            case "INSERT":
            {
                var o = Pair(); var whole = (object[])o[0]; var arg = o[1];
                if (whole.Length < 2 || arg is not object[] xs || xs.Length == 0)
                    return Bot.Value;
                var acc = xs[^1];
                for (int i = xs.Length - 2; i >= 0; i--)
                    acc = Mu(App(whole[1], MkSeq(new[] { xs[i], acc })));
                return acc;
            }
            case "WHILE":
            {
                var o = Pair(); var whole = (object[])o[0]; var arg = o[1];
                if (whole.Length < 3) return Bot.Value;
                while (true)
                {
                    var pv = Mu(App(whole[1], arg));
                    if (Equals(pv, F)) return arg;
                    if (!Equals(pv, T)) return Bot.Value;
                    arg = Mu(App(whole[2], arg));
                }
            }
            case "BU":
            {
                var o = Pair(); var whole = (object[])o[0]; var arg = o[1];
                if (whole.Length < 3) return Bot.Value;
                next = true;
                return App(whole[1], new object[] { whole[2], arg });
            }
            case "id": return x;
            case "tl": return x is object[] s1 && s1.Length >= 1
                              ? s1.Skip(1).ToArray() : Bot.Value;
            case "atom": return ReferenceEquals(x, Bot.Value) ? Bot.Value
                              : (x is object[] s2 && s2.Length > 0 ? F : T);
            case "null": return ReferenceEquals(x, Bot.Value) ? Bot.Value
                              : (x is object[] { Length: 0 } ? T : F);
            case "eq":
            {
                var o = Pair();
                return o is { Length: 2 } ? (EqObj(o[0], o[1]) ? T : F) : Bot.Value;
            }
            case "apndl":
            {
                var o = Pair();
                if (o is { Length: 2 } && o[1] is object[] ys)
                    return new[] { o[0] }.Concat(ys).ToArray();
                return Bot.Value;
            }
            case "apndr":
            {
                var o = Pair();
                if (o is { Length: 2 } && o[0] is object[] ys)
                    return ys.Concat(new[] { o[1] }).ToArray();
                return Bot.Value;
            }
            case "distl":
            {
                var o = Pair();
                if (o is { Length: 2 } && o[1] is object[] ys)
                    return ys.Select(y => (object)new[] { o[0], y }).ToArray();
                return Bot.Value;
            }
            case "distr":
            {
                var o = Pair();
                if (o is { Length: 2 } && o[0] is object[] xs2)
                    return xs2.Select(xx => (object)new[] { xx, o[1] }).ToArray();
                return Bot.Value;
            }
            case "length": return x is object[] s3 ? (long)s3.Length : Bot.Value;
            case "reverse": return x is object[] s4 ? s4.Reverse().ToArray() : Bot.Value;
            case "cat":
            {
                var o = Pair();
                if (o is { Length: 2 } && o[0] is object[] a4 && o[1] is object[] b4)
                    return a4.Concat(b4).ToArray();
                return Bot.Value;
            }
            case "not": return Equals(x, T) ? F : (Equals(x, F) ? T : Bot.Value);
            case "and":
            {
                var o = Pair();
                if (o is { Length: 2 } && (Equals(o[0], T) || Equals(o[0], F))
                                       && (Equals(o[1], T) || Equals(o[1], F)))
                    return Equals(o[0], T) && Equals(o[1], T) ? T : F;
                return Bot.Value;
            }
            case "or":
            {
                var o = Pair();
                if (o is { Length: 2 } && (Equals(o[0], T) || Equals(o[0], F))
                                       && (Equals(o[1], T) || Equals(o[1], F)))
                    return Equals(o[0], T) || Equals(o[1], T) ? T : F;
                return Bot.Value;
            }
            case "1r": return x is object[] s5 && s5.Length >= 1 ? s5[^1] : Bot.Value;
            case "tlr": return x is object[] s6 && s6.Length >= 1
                               ? s6.Take(s6.Length - 1).ToArray() : Bot.Value;
            case "rotl": return x is object[] s7 && s7.Length > 0
                               ? s7.Skip(1).Concat(s7.Take(1)).ToArray()
                               : (x is object[] e7 ? e7 : Bot.Value);
            case "rotr": return x is object[] s8 && s8.Length > 0
                               ? s8.Skip(s8.Length - 1).Concat(s8.Take(s8.Length - 1)).ToArray()
                               : (x is object[] e8 ? e8 : Bot.Value);
            case "trans":
            {
                if (x is not object[] rows || rows.Any(r => r is not object[]))
                    return Bot.Value;
                if (rows.Length == 0) return Array.Empty<object>();
                var w = ((object[])rows[0]).Length;
                if (rows.Any(r => ((object[])r).Length != w)) return Bot.Value;
                return Enumerable.Range(0, w).Select(i =>
                    (object)rows.Select(r => ((object[])r)[i]).ToArray()).ToArray();
            }
            case "+":
            {
                var o = Pair();
                return o is { Length: 2 }
                    ? Arith(o[0], o[1], (a, b) => a + b, (a, b) => a + b) : Bot.Value;
            }
            case "-":
            {
                var o = Pair();
                return o is { Length: 2 }
                    ? Arith(o[0], o[1], (a, b) => a - b, (a, b) => a - b) : Bot.Value;
            }
            case "*":
            {
                var o = Pair();
                return o is { Length: 2 }
                    ? Arith(o[0], o[1], (a, b) => a * b, (a, b) => a * b) : Bot.Value;
            }
            case "div":
            {
                var o = Pair();
                if (o is { Length: 2 })
                {
                    var na = ToNum(o[0]); var nb = ToNum(o[1]);
                    if (o[0] is not string && o[1] is not string &&
                        na is double da && nb is double db && db != 0)
                        return da / db;
                }
                return Bot.Value;
            }
            case "ge": { var o = Pair(); return o is { Length: 2 } ? Cmp(o[0], o[1], (a, b) => a >= b, (a, b) => string.CompareOrdinal(a, b) >= 0) : Bot.Value; }
            case "gt": { var o = Pair(); return o is { Length: 2 } ? Cmp(o[0], o[1], (a, b) => a > b, (a, b) => string.CompareOrdinal(a, b) > 0) : Bot.Value; }
            case "le": { var o = Pair(); return o is { Length: 2 } ? Cmp(o[0], o[1], (a, b) => a <= b, (a, b) => string.CompareOrdinal(a, b) <= 0) : Bot.Value; }
            case "lt": { var o = Pair(); return o is { Length: 2 } ? Cmp(o[0], o[1], (a, b) => a < b, (a, b) => string.CompareOrdinal(a, b) < 0) : Bot.Value; }
            case "apply":
            {
                var o = Pair();
                if (o is { Length: 2 }) { next = true; return App(o[0], o[1]); }
                return Bot.Value;
            }
            default: handled = false; return null;
        }
    }
}
