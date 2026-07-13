// Spec v3: a host is a store loader and a mu. The canon DEFs boot from
// shared/canon.store.json and the cross-host case table from
// shared/scenarios.store.json, each a CELL row <"CELL", name, term> in the
// native encoding (an atom is a String/Long, a sequence is an Object[]),
// built once for every host by tools/build_canon_store.py. This host
// therefore needs no vocabulary binding, no generated wrap, and no
// generator: the JSON reader below and the reducer are the whole host.
// Retires gen_canon.py and the generated Canon.java (2026-07-13).
package arest;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;

public class StoreCanon {

    static String sharedPath(String name) {
        String dir = System.getProperty("arest.shared", "../shared");
        return dir + "/" + name;
    }

    /** The canon DEFs, in store order: pairs of name and term. */
    public static List<Object[]> loadAll() {
        return loadDefs("canon.store.json");
    }

    /** The cross-host scenario cases, same shape. */
    public static List<Object[]> loadScenarioDefs() {
        return loadDefs("scenarios.store.json");
    }

    static List<Object[]> loadDefs(String file) {
        String text;
        try {
            text = new String(
                Files.readAllBytes(Paths.get(sharedPath(file))),
                StandardCharsets.UTF_8);
        } catch (IOException e) {
            throw new RuntimeException("cannot read " + sharedPath(file), e);
        }
        Object doc = new Json(text).parseValue();
        Object d = null;
        if (doc instanceof java.util.Map) {
            d = ((java.util.Map<?, ?>) doc).get("d");
        }
        if (!(d instanceof Object[])) {
            throw new RuntimeException(file + " carries no d sequence");
        }
        List<Object[]> defs = new ArrayList<Object[]>();
        for (Object cell : (Object[]) d) {
            if (!(cell instanceof Object[])) {
                continue;
            }
            Object[] c = (Object[]) cell;
            if (c.length >= 3 && "CELL".equals(c[0])) {
                defs.add(new Object[] { c[1], c[2] });
            }
        }
        return defs;
    }

    /** A minimal recursive-descent JSON reader over the store grammar:
     *  objects, arrays, strings (escapes and \\uXXXX), numbers (Long when
     *  integral, Double otherwise), true, false, null. Arrays land as
     *  Object[] and integers as Long, which IS the reducer's native term
     *  encoding, so no second mapping pass exists. */
    static final class Json {
        private final String s;
        private int i;

        Json(String text) {
            this.s = text;
            this.i = 0;
        }

        Object parseValue() {
            ws();
            char c = s.charAt(i);
            if (c == '{') return object();
            if (c == '[') return array();
            if (c == '"') return string();
            if (c == 't') { i += 4; return Boolean.TRUE; }
            if (c == 'f') { i += 5; return Boolean.FALSE; }
            if (c == 'n') { i += 4; return null; }
            return number();
        }

        private void ws() {
            while (i < s.length() && Character.isWhitespace(s.charAt(i))) i++;
        }

        private java.util.Map<String, Object> object() {
            java.util.Map<String, Object> m =
                new java.util.LinkedHashMap<String, Object>();
            i++; // {
            ws();
            if (s.charAt(i) == '}') { i++; return m; }
            while (true) {
                ws();
                String k = string();
                ws();
                i++; // :
                m.put(k, parseValue());
                ws();
                if (s.charAt(i) == ',') { i++; continue; }
                i++; // }
                return m;
            }
        }

        private Object[] array() {
            List<Object> out = new ArrayList<Object>();
            i++; // [
            ws();
            if (s.charAt(i) == ']') { i++; return out.toArray(); }
            while (true) {
                out.add(parseValue());
                ws();
                if (s.charAt(i) == ',') { i++; ws(); continue; }
                i++; // ]
                return out.toArray();
            }
        }

        private String string() {
            StringBuilder sb = new StringBuilder();
            i++; // opening quote
            while (true) {
                char c = s.charAt(i++);
                if (c == '"') return sb.toString();
                if (c != '\\') { sb.append(c); continue; }
                char e = s.charAt(i++);
                switch (e) {
                    case '"': sb.append('"'); break;
                    case '\\': sb.append('\\'); break;
                    case '/': sb.append('/'); break;
                    case 'b': sb.append('\b'); break;
                    case 'f': sb.append('\f'); break;
                    case 'n': sb.append('\n'); break;
                    case 'r': sb.append('\r'); break;
                    case 't': sb.append('\t'); break;
                    case 'u':
                        sb.append((char) Integer.parseInt(
                            s.substring(i, i + 4), 16));
                        i += 4;
                        break;
                    default: sb.append(e);
                }
            }
        }

        private Object number() {
            int start = i;
            boolean integral = true;
            if (s.charAt(i) == '-') i++;
            while (i < s.length()) {
                char c = s.charAt(i);
                if (c >= '0' && c <= '9') { i++; continue; }
                if (c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-') {
                    integral = false;
                    i++;
                    continue;
                }
                break;
            }
            String t = s.substring(start, i);
            return integral
                ? (Object) Long.valueOf(Long.parseLong(t))
                : (Object) Double.valueOf(Double.parseDouble(t));
        }
    }
}
