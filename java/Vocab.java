// The Java binding of the INTERSECTION SOURCE vocabulary
// (shared/intersection.md): the same names CPython execs, rustc tokenizes,
// and the C# host wraps. The generated Canon.java static-imports these, so
// each shared file's bytes become one varargs method call (return T <file>;).
// Trees build in the delta evaluator's native form: an atom is a scalar
// (String/Long), a sequence is an Object[], K(x) is the pair ("CONST", x).
package arestlam;

import java.util.ArrayList;
import java.util.List;

public class Vocab {
    public static final List<Object[]> DEFS = new ArrayList<Object[]>();

    public static Object T(Object... elements) { return elements; }

    public static Object DEF(String name, Object tree) {
        DEFS.add(new Object[] { name, tree });
        return name;
    }

    public static Object A(String s) { return s; }
    public static Object A(long i) { return Long.valueOf(i); }
    public static Object N(long i) { return Long.valueOf(i); }
    public static Object PHI() { return new Object[0]; }
    public static Object K(Object x) { return new Object[] { "CONST", x }; }

    public static Object S1(Object a) { return new Object[] { a }; }
    public static Object S2(Object a, Object b) { return new Object[] { a, b }; }
    public static Object S3(Object a, Object b, Object c) {
        return new Object[] { a, b, c };
    }
    public static Object S4(Object a, Object b, Object c, Object d) {
        return new Object[] { a, b, c, d };
    }
    public static Object S5(Object a, Object b, Object c, Object d, Object e) {
        return new Object[] { a, b, c, d, e };
    }
    public static Object S6(Object a, Object b, Object c, Object d, Object e,
                            Object f) {
        return new Object[] { a, b, c, d, e, f };
    }
    public static Object S7(Object a, Object b, Object c, Object d, Object e,
                            Object f, Object g) {
        return new Object[] { a, b, c, d, e, f, g };
    }
    public static Object S8(Object a, Object b, Object c, Object d, Object e,
                            Object f, Object g, Object h) {
        return new Object[] { a, b, c, d, e, f, g, h };
    }
    public static Object S9(Object a, Object b, Object c, Object d, Object e,
                            Object f, Object g, Object h, Object i) {
        return new Object[] { a, b, c, d, e, f, g, h, i };
    }
}
