PROGRAM ThresholdAlarm
VAR_INPUT
    D0 : INT;
    D1 : INT;
END_VAR
VAR_OUTPUT
    Y0 : BOOL;
END_VAR
VAR
    __t1 : INT;
END_VAR

    __t1 := D0 + D1;
    Y0 := __t1 > 100;
END_PROGRAM