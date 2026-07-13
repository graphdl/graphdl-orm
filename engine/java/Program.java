// The Java host's first breath: load the canon through the byte-wrapped
// vocabulary and report what landed. The reducer follows; the acceptance is
// the cross-host differential over the same probes the other kernels agree on.
package arest;

import java.util.List;

public class Program {
    static Object seq(Object... xs) { return xs; }

    static String show(Object o) {
        if (o == Reducer.BOT) return "⊥";
        if (o instanceof Object[]) {
            StringBuilder sb = new StringBuilder("(");
            Object[] s = (Object[]) o;
            for (int i = 0; i < s.length; i++) {
                if (i > 0) sb.append(", ");
                sb.append(show(s[i]));
            }
            return sb.append(")").toString();
        }
        if (o instanceof String) return "'" + o + "'";
        return String.valueOf(o);
    }

    public static void main(String[] args) {
        Reducer.loadCanon();
        System.out.println("defs=" + Reducer.STORE.size());

        Object max2 = Reducer.mu(Reducer.app("system:max2", seq("305", "1190")));
        System.out.println("max2('305','1190')=" + show(max2));

        Object pops = seq(
            seq(seq("t1", "draft"), seq("t2", "review")),
            seq(seq("t1", "submit"), seq("t2", "approve")),
            seq(seq("t1", "review"), seq("t2", "done")));
        System.out.println("sm_join="
            + show(Reducer.mu(Reducer.app("system:sm_join", pops))));

        Object mint = Reducer.mu(Reducer.app(
            Reducer.mu(Reducer.app("system:mint_next", Long.valueOf(1))),
            seq(seq("2", "x"), seq("7", "y"), seq("4", "z"))));
        System.out.println("mint_next=(8 expected) " + show(mint));

        Object rule = Reducer.mu(Reducer.app("system:join_rule",
            seq(Long.valueOf(2), seq(Long.valueOf(1), Long.valueOf(3)))));
        Object closure = Reducer.mu(Reducer.app(
            Reducer.mu(Reducer.app("system:derive_of", seq(rule))),
            seq(seq("a", "b"), seq("b", "c"), seq("c", "d"))));
        System.out.println("closure=" + show(closure));

        // the cross-host case table (shared/scenarios.canon, booted from the
        // store artifact): each case is a pair of expr and operand; reduce
        // and print for the differential
        for (Object[] kv : StoreCanon.loadScenarioDefs()) {
            Object[] pair = (Object[]) kv[1];
            Object got = Reducer.mu(Reducer.app(pair[0], pair[1]));
            System.out.println(kv[0] + "=" + show(got));
        }
    }
}
