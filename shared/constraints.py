(
"Constraint families as canonical definitions in INTERSECTION SOURCE (one tuple literal, normal Python and normal Rust verbatim; see shared/theta.py for the discipline). The closed families are references into theta; uniqueness is a higher-order builder applied to the key roles, and exclusion is uniqueness applied to the entity role through the apply primitive (dynamic application, the base's own higher-order mechanism).",

DEF("constraints:mandatory", A("theta:setminus")),

DEF("constraints:subset", A("theta:setminus")),

DEF("constraints:inclusive_or", A("theta:setminus")),

DEF("constraints:equality",
    S3(A("COMP"), A("cat"),
       S3(A("CONS"),
          S3(A("COMP"), A("theta:setminus"), S3(A("CONS"), N(1), N(2))),
          S3(A("COMP"), A("theta:setminus"), S3(A("CONS"), N(2), N(1)))))),

DEF("constraints:uniqueness",
    S6(A("CONS"), K(A("COMP")),
       K(S2(A("ALPHA"), N(1))),
       S3(A("COMP"), A("theta:Filter"),
          S6(A("CONS"), K(A("COMP")), K(A("not")), K(A("null")),
             S3(A("COMP"), A("theta:Filter"),
                S4(A("CONS"), K(A("COMP")), K(A("and")),
                   S4(A("CONS"), K(A("CONS")),
                      S4(A("CONS"), K(A("COMP")), K(A("eq")),
                         S4(A("CONS"), K(A("CONS")),
                            S4(A("CONS"), K(A("COMP")), A("theta:selrow"), K(N(1))),
                            S4(A("CONS"), K(A("COMP")), A("theta:selrow"), K(N(2))))),
                      K(S3(A("COMP"), A("not"), A("eq")))))),
             K(A("distl")))),
       K(A("distr")),
       K(S3(A("CONS"), A("id"), A("id"))))),

DEF("constraints:exclusion",
    S3(A("COMP"), A("apply"),
       S3(A("CONS"),
          S3(A("COMP"), A("constraints:uniqueness"), K(S1(N(1)))),
          A("id")))),
)
