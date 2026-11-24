PROGRAM MotorStartStop
VAR_INPUT
    M0 : BOOL;
    X0 : BOOL;
    X1 : BOOL;
END_VAR
VAR_OUTPUT
    Y0 : BOOL;
END_VAR
VAR
    __t1 : BOOL;
END_VAR

    __t1 := X0 OR M0;
    Y0 := __t1 AND X1;
END_PROGRAM