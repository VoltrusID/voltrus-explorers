PROGRAM Counter
VAR_INPUT
    X0 : BOOL;
    X1 : BOOL;
END_VAR
VAR_OUTPUT
    Y0 : BOOL;
END_VAR
VAR
    part_counter : CTU;
END_VAR

    part_counter(CU := X0, R := X1, PV := 10);
    Y0 := part_counter.Q;
END_PROGRAM