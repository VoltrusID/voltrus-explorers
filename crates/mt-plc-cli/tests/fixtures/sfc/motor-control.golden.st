PROGRAM MotorControl
VAR
    _s1 : BOOL := TRUE;
    _s2 : BOOL;
    _s3 : BOOL;
    _s4 : BOOL;
    _s1_prev : BOOL;
    _s2_prev : BOOL;
    _s3_prev : BOOL;
    _s4_prev : BOOL;
    Motor : BOOL;
    Counter : BOOL;
    RunLamp : BOOL;
END_VAR

    _s1_prev := _s1;
    _s2_prev := _s2;
    _s3_prev := _s3;
    _s4_prev := _s4;
    IF _s1 AND (StartButton) THEN
        _s1 := FALSE;
        _s2 := TRUE;
    END_IF;
    IF _s2 AND (MotorReady) THEN
        _s2 := FALSE;
        _s3 := TRUE;
    END_IF;
    IF _s3 AND (StopButton) THEN
        _s3 := FALSE;
        _s4 := TRUE;
    END_IF;
    IF _s4 AND (MotorStopped) THEN
        _s4 := FALSE;
        _s1 := TRUE;
    END_IF;
    IF _s2 AND NOT _s2_prev THEN Motor := TRUE; END_IF;
    IF _s2 AND NOT _s2_prev THEN Counter := Counter + 1; END_IF;
    Motor := _s3;
    RunLamp := _s3;
    IF _s4 AND NOT _s4_prev THEN Motor := FALSE; END_IF;
END_PROGRAM