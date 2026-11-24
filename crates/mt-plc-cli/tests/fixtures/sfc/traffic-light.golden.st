PROGRAM TrafficLight
VAR
    _s1 : BOOL := TRUE;
    _s2 : BOOL;
    _s3 : BOOL;
    _s1_prev : BOOL;
    _s2_prev : BOOL;
    _s3_prev : BOOL;
    GreenLamp : BOOL;
    YellowLamp : BOOL;
    RedLamp : BOOL;
END_VAR

    _s1_prev := _s1;
    _s2_prev := _s2;
    _s3_prev := _s3;
    IF _s1 AND (T1.Q) THEN
        _s1 := FALSE;
        _s2 := TRUE;
    END_IF;
    IF _s2 AND (T2.Q) THEN
        _s2 := FALSE;
        _s3 := TRUE;
    END_IF;
    IF _s3 AND (T3.Q) THEN
        _s3 := FALSE;
        _s1 := TRUE;
    END_IF;
    GreenLamp := _s1;
    YellowLamp := _s2;
    RedLamp := _s3;
END_PROGRAM