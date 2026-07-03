(
"theta1 (Codd 2.2) as canonical definitions in INTERSECTION SOURCE: this file is a normal Python module and, include!d in expression position, normal Rust. It is ONE tuple literal whose elements evaluate left to right in both languages; the vocabulary (DEF, A, N, PHI, S2..S9) is defined by each platform, so the lambda bound determines the implementation. Definitions reference each other by NAME, resolved through DEFS by the one mu (Backus 13.3.5): no host functions, no assignments, no imports, double-quoted strings only.",

DEF("theta:append_phi",
    S3(A("COMP"), A("apndr"), S3(A("CONS"), A("id"), S2(A("CONST"), PHI)))),

DEF("theta:keep_eq",
    S4(A("COND"), S3(A("COMP"), A("eq"), N(1)), A("apndl"), N(2))),

DEF("theta:filter_eq",
    S3(A("COMP"), S2(A("INSERT"), A("theta:keep_eq")), A("theta:append_phi"))),

DEF("theta:member",
    S5(A("COMP"), A("not"), A("null"), A("theta:filter_eq"), A("distl"))),

DEF("theta:dedup",
    S3(A("COMP"),
       S2(A("INSERT"), S4(A("COND"), A("theta:member"), N(2), A("apndl"))),
       A("theta:append_phi"))),

DEF("theta:flatten",
    S3(A("COMP"), S2(A("INSERT"), A("cat")), A("theta:append_phi"))),

DEF("theta:keep_notmember",
    S4(A("COND"),
       S3(A("COMP"), S3(A("COMP"), A("not"), A("theta:member")), N(1)),
       A("apndl"), N(2))),

DEF("theta:setminus",
    S4(A("COMP"), S2(A("ALPHA"), N(1)),
       S3(A("COMP"), S2(A("INSERT"), A("theta:keep_notmember")),
          A("theta:append_phi")),
       A("distr"))),

DEF("theta:tie_keep",
    S4(A("COND"),
       S3(A("COMP"), S3(A("COMP"), A("eq"), S3(A("CONS"), N(1), A("1r"))), N(1)),
       A("apndl"), N(2))),

DEF("theta:Tie",
    S3(A("COMP"), S2(A("ALPHA"), A("tlr")),
       S3(A("COMP"), S2(A("INSERT"), A("theta:tie_keep")),
          A("theta:append_phi")))),
)
