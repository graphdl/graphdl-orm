// Spec v3: a host is a store loader and a mu. The canon DEFs boot from
// shared/canon.store.json and the cross-host case table from
// shared/scenarios.store.json, each a CELL row <"CELL", name, term> in the
// native encoding (an atom is a string/long, a sequence is an object[]),
// built once for every host by tools/build_canon_store.py. This host
// therefore needs no vocabulary binding, no MSBuild byte-wrap, and no
// generated Canon.g.cs: System.Text.Json (base library, zero-dep) maps
// arrays and integers directly onto the reducer's carrier. Retires the
// csproj WrapCanon target and Vocabulary.cs (2026-07-13), mirroring the
// Java host's StoreCanon.
namespace Arestlam;

using System.Text.Json;

static class StoreCanon
{
    static string SharedPath(string name)
    {
        var env = Environment.GetEnvironmentVariable("AREST_SHARED");
        if (env != null) return Path.Combine(env, name);
        // dotnet run leaves the caller's cwd in place, so resolve from the
        // binary's own home upward to the engine root's shared/
        var dir = AppContext.BaseDirectory;
        for (var i = 0; i < 8 && dir != null; i++)
        {
            var probe = Path.Combine(dir, "shared", name);
            if (File.Exists(probe)) return probe;
            dir = Path.GetDirectoryName(dir.TrimEnd(Path.DirectorySeparatorChar));
        }
        return Path.Combine("..", "shared", name);
    }

    internal static List<KeyValuePair<string, object>> LoadAll()
        => LoadDefs("canon.store.json");

    internal static List<KeyValuePair<string, object>> LoadScenarioDefs()
        => LoadDefs("scenarios.store.json");

    static List<KeyValuePair<string, object>> LoadDefs(string file)
    {
        using var doc = JsonDocument.Parse(File.ReadAllText(SharedPath(file)));
        var defs = new List<KeyValuePair<string, object>>();
        foreach (var cell in doc.RootElement.GetProperty("d").EnumerateArray())
        {
            if (cell.ValueKind != JsonValueKind.Array) continue;
            var c = new List<JsonElement>(cell.EnumerateArray());
            if (c.Count >= 3 && c[0].ValueKind == JsonValueKind.String
                && c[0].GetString() == "CELL"
                && c[1].ValueKind == JsonValueKind.String)
                defs.Add(new KeyValuePair<string, object>(
                    c[1].GetString()!, Term(c[2])));
        }
        return defs;
    }

    // arrays land as object[] and integers as long, which IS the reducer's
    // native term encoding, so no second mapping pass exists
    static object Term(JsonElement e)
    {
        switch (e.ValueKind)
        {
            case JsonValueKind.Array:
                var items = new List<object>();
                foreach (var x in e.EnumerateArray()) items.Add(Term(x));
                return items.ToArray();
            case JsonValueKind.String:
                return e.GetString()!;
            case JsonValueKind.Number:
                // two returns, never one conditional: the ternary would
                // unify long and double to double and box every selector
                // as a float (found 2026-07-13, every selector bottomed)
                if (e.TryGetInt64(out var i)) return i;
                return e.GetDouble();
            case JsonValueKind.True:
                return true;
            case JsonValueKind.False:
                return false;
            default:
                return Array.Empty<object>();
        }
    }
}
