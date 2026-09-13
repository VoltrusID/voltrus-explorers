PROGRAM TimerDelay
VAR_INPUT
    X0 : BOOL;
END_VAR
VAR_OUTPUT
    Y0 : BOOL;
END_VAR
VAR
    delay_timer : TON;
END_VAR

    delay_timer(IN := X0, PT := T#500MS);
    Y0 := delay_timer.Q;
END_PROGRAM