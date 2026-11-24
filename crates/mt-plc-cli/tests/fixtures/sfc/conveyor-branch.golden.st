PROGRAM ConveyorBranch
VAR
    _s1 : BOOL := TRUE;
    _s2 : BOOL;
    _s3 : BOOL;
    _s4 : BOOL;
    _s1_prev : BOOL;
    _s2_prev : BOOL;
    _s3_prev : BOOL;
    _s4_prev : BOOL;
    ConveyorMotor : BOOL;
    AlarmLamp : BOOL;
END_VAR

    _s1_prev := _s1;
    _s2_prev := _s2;
    _s3_prev := _s3;
    _s4_prev := _s4;
    IF _s1 AND (StartButton) THEN
        _s1 := FALSE;
        _s2 := TRUE;
    END_IF;
    IF _s2 AND (ResetButton) THEN
        _s2 := FALSE;
        _s4 := TRUE;
    END_IF;
    IF _s2 AND (ResetButton) THEN
        _s2 := FALSE;
        _s4 := TRUE;
    END_IF;
    IF _s3 AND (ResetButton) THEN
        _s3 := FALSE;
        _s4 := TRUE;
    END_IF;
    IF _s4 AND (TRUE) THEN
        _s4 := FALSE;
        _s1 := TRUE;
    END_IF;
    ConveyorMotor := _s2;
    IF _s3 AND NOT _s3_prev THEN AlarmLamp := TRUE; END_IF;
    IF _s4 AND NOT _s4_prev THEN AlarmLamp := FALSE; END_IF;
END_PROGRAM