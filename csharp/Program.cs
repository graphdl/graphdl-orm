// The C# host's acceptance probes: load the canon, reduce the same
// applications the Python twin tests pin, and print rows in a stable form
// the cross-host differential compares byte-for-byte.
namespace Arestlam;

static class Program
{
    static object Seq(params object[] xs) => xs;

    static string Show(object o)
    {
        if (ReferenceEquals(o, Bot.Value)) return "⊥";
        if (o is object[] s) return "(" + string.Join(", ", s.Select(Show)) + ")";
        if (o is string str) return "'" + str + "'";
        if (o is double d)
            // Python repr: a whole double still shows its .0
            return d == Math.Floor(d) && !double.IsInfinity(d)
                ? d.ToString("0.0") : d.ToString();
        return o.ToString() ?? "null";
    }

    static int Main()
    {
        Reducer.LoadCanon();
        Console.WriteLine($"defs={Reducer.Store.Count}");

        // system:max2 over a pair (coerced comparison underneath)
        var max2 = Reducer.Mu(Reducer.App("system:max2", Seq("305", "1190")));
        Console.WriteLine($"max2('305','1190')={Show(max2)}");

        // system:sm_join over the twin test's machine
        var pops = Seq(
            Seq(Seq("t1", "draft"), Seq("t2", "review")),
            Seq(Seq("t1", "submit"), Seq("t2", "approve")),
            Seq(Seq("t1", "review"), Seq("t2", "done")));
        var sm = Reducer.Mu(Reducer.App("system:sm_join", pops));
        Console.WriteLine($"sm_join={Show(sm)}");

        // system:mint_next at column 1 (max fold + successor; empty answers 1)
        var mint = Reducer.Mu(Reducer.App(
            Reducer.Mu(Reducer.App("system:mint_next", 1L)),
            Seq(Seq("2", "x"), Seq("7", "y"), Seq("4", "z"))));
        Console.WriteLine($"mint_next=(8 expected) {Show(mint)}");

        // system:derive_of over the transitive-closure twin case
        var rule = Reducer.Mu(Reducer.App("system:join_rule", Seq(2L, Seq(1L, 3L))));
        var closure = Reducer.Mu(Reducer.App(
            Reducer.Mu(Reducer.App("system:derive_of", Seq(rule))),
            Seq(Seq("a", "b"), Seq("b", "c"), Seq("c", "d"))));
        Console.WriteLine($"closure={Show(closure)}");

        // the cross-host case table (shared/scenarios.py): each case is
        // ⟨expr, operand⟩; reduce and print for the differential
        Canon.Defs.Clear();
        Canon.LoadScenarios();
        foreach (var kv in Canon.Defs.ToList())
        {
            var pair = (object[])kv.Value;
            var got = Reducer.Mu(Reducer.App(pair[0], pair[1]));
            Console.WriteLine($"{kv.Key}={Show(got)}");
        }
        return 0;
    }
}
