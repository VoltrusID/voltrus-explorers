PROGRAM ESP32_RelayChase
VAR
    // buzzer : BOOL := FALSE;
    red : DINT := 0;
    green : DINT := 0;
    blue : DINT := 0;
    relay0 : BOOL := FALSE;
    relay1 : BOOL := FALSE;
    relay2 : BOOL := FALSE;
    relay3 : BOOL := FALSE;
    relay4 : BOOL := FALSE;
    relay5 : BOOL := FALSE;
    phase : DINT := 0;
    step_timer : TON;
END_VAR
    step_timer(IN := TRUE, PT := T#2S);
    IF step_timer.Q THEN
        step_timer(IN := FALSE, PT := T#0MS);
        phase := phase + 1;
        // IF phase > 10 THEN
        //     phase := 0;
        //     buzzer := NOT buzzer;
        // END_IF;
    END_IF;

    relay0 := FALSE; relay1 := FALSE; relay2 := FALSE;
    relay3 := FALSE; relay4 := FALSE; relay5 := FALSE;

    CASE phase OF
        0: relay0 := TRUE; red := 255; green := 0;   blue := 0;
        1: relay1 := TRUE; red := 0;   green := 255; blue := 0;
        2: relay2 := TRUE; red := 0;   green := 0;   blue := 255;
        3: relay3 := TRUE; red := 255; green := 255; blue := 0;
        4: relay4 := TRUE; red := 255; green := 0;   blue := 255;
        5: relay5 := TRUE; red := 0;   green := 255; blue := 255;
    END_CASE;
END_PROGRAM