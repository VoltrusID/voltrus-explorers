PROGRAM ESP32_Tester
VAR
    (* Inputs from ESP32 — mapped to Modbus holding registers *)
    counter : DINT;        (* Reg 0: auto-incrementing counter, 1s interval *)
    random_val : DINT;     (* Reg 1: random value 0-999 *)

    (* Outputs to ESP32 — mapped to Modbus holding registers *)
    buzzer : BOOL := FALSE;     (* Reg 10: buzzer on/off *)
    red : DINT := 0;            (* Reg 11: RGB red 0-255 *)
    green : DINT := 0;          (* Reg 12: RGB green 0-255 *)
    blue : DINT := 0;           (* Reg 13: RGB blue 0-255 *)

    (* Relay outputs — mapped to Modbus coils 0-5 *)
    relay0 : BOOL := FALSE;
    relay1 : BOOL := FALSE;
    relay2 : BOOL := FALSE;
    relay3 : BOOL := FALSE;
    relay4 : BOOL := FALSE;
    relay5 : BOOL := FALSE;

    (* Internal logic *)
    cycle : DINT := 0;
    phase : DINT := 0;
    step_timer : TON;
    flash_timer : TON;
END_VAR

    cycle := cycle + 1;

    (* Step timer: advance phase every 2 seconds *)
    step_timer(IN := TRUE, PT := T#2S);
    IF step_timer.Q THEN
        step_timer(IN := FALSE, PT := T#0MS);
        phase := phase + 1;
        IF phase > 5 THEN
            phase := 0;
        END_IF;
    END_IF;

    (* Flash timer for visual feedback *)
    flash_timer(IN := TRUE, PT := T#500MS);
    IF flash_timer.Q THEN
        flash_timer(IN := FALSE, PT := T#0MS);
    END_IF;

    (* Buzzer: beep when counter is in upper half *)
    IF counter > 500 THEN
        buzzer := TRUE;
    ELSE
        buzzer := FALSE;
    END_IF;

    (* Relay pattern based on phase *)
    CASE phase OF
        0:  (* All off *)
            relay0 := FALSE; relay1 := FALSE; relay2 := FALSE;
            relay3 := FALSE; relay4 := FALSE; relay5 := FALSE;
        1:  (* Chase right *)
            relay0 := TRUE; relay1 := FALSE; relay2 := FALSE;
            relay3 := FALSE; relay4 := FALSE; relay5 := FALSE;
        2:  (* Chase right *)
            relay0 := FALSE; relay1 := TRUE; relay2 := FALSE;
            relay3 := FALSE; relay4 := FALSE; relay5 := FALSE;
        3:  (* Chase right *)
            relay0 := FALSE; relay1 := FALSE; relay2 := TRUE;
            relay3 := FALSE; relay4 := FALSE; relay5 := FALSE;
        4:  (* Chase right *)
            relay0 := FALSE; relay1 := FALSE; relay2 := FALSE;
            relay3 := TRUE; relay4 := FALSE; relay5 := FALSE;
        5:  (* All on *)
            relay0 := TRUE; relay1 := TRUE; relay2 := TRUE;
            relay3 := TRUE; relay4 := TRUE; relay5 := TRUE;
    END_CASE;

    (* RGB LED: cycle through colors based on phase *)
    CASE phase OF
        0:  red := 255; green := 0;   blue := 0;     (* Red *)
        1:  red := 0;   green := 255; blue := 0;     (* Green *)
        2:  red := 0;   green := 0;   blue := 255;   (* Blue *)
        3:  red := 255; green := 255; blue := 0;     (* Yellow *)
        4:  red := 255; green := 0;   blue := 255;   (* Magenta *)
        5:  red := 0;   green := 255; blue := 255;   (* Cyan *)
    END_CASE;

END_PROGRAM
