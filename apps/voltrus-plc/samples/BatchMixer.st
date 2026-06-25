PROGRAM BatchMixer
VAR
    (* State machine *)
    state : INT := 0;
    step_timer : INT := 0;

    (* Ingredient valves *)
    valve_a : BOOL := FALSE;
    valve_b : BOOL := FALSE;

    (* Process outputs *)
    mixer : BOOL := FALSE;
    heater : BOOL := FALSE;
    drain : BOOL := FALSE;

    (* Sensors *)
    level : REAL := 0.0;
    temperature : REAL := 20.0;

    (* Product *)
    batch_count : INT := 0;
    total_drained : REAL := 0.0;

    (* Recipe setpoints *)
    fill_level_a : REAL := 35.0;
    fill_level_b : REAL := 70.0;
    target_temp : REAL := 75.0;
    mix_time : INT := 150;
    drain_time : INT := 100;

    (* Alarms *)
    fault : BOOL := FALSE;
    fault_msg : INT := 0;

    (* Cycle counter *)
    cycles : DINT := 0;
END_VAR

    cycles := cycles + 1;
    step_timer := step_timer + 1;

    (* State machine *)
    IF state = 0 THEN
        (* IDLE *)
        valve_a := FALSE;
        valve_b := FALSE;
        mixer := FALSE;
        heater := FALSE;
        drain := FALSE;
        step_timer := 0;
        IF level <= 0.0 AND temperature < 30.0 THEN
            state := 1;
        END_IF;

    ELSIF state = 1 THEN
        (* FILL ingredient A *)
        valve_a := TRUE;
        IF level >= fill_level_a THEN
            valve_a := FALSE;
            state := 2;
            step_timer := 0;
        END_IF;

    ELSIF state = 2 THEN
        (* FILL ingredient B *)
        valve_b := TRUE;
        IF level >= fill_level_b THEN
            valve_b := FALSE;
            state := 3;
            step_timer := 0;
        END_IF;

    ELSIF state = 3 THEN
        (* HEAT to target *)
        mixer := TRUE;
        heater := TRUE;
        IF temperature >= target_temp THEN
            heater := FALSE;
            state := 4;
            step_timer := 0;
        END_IF;

    ELSIF state = 4 THEN
        (* MIX for duration *)
        mixer := TRUE;
        IF step_timer >= mix_time THEN
            mixer := FALSE;
            state := 5;
            step_timer := 0;
        END_IF;

    ELSIF state = 5 THEN
        (* DRAIN product *)
        drain := TRUE;
        IF step_timer >= drain_time THEN
            drain := FALSE;
            batch_count := batch_count + 1;
            state := 0;
            step_timer := 0;
        END_IF;

    END_IF;

    (* Watchdog: fault if any state takes too long *)
    IF step_timer > 500 THEN
        fault := TRUE;
        fault_msg := state;
    END_IF;

    (* Simulate process dynamics *)
    IF valve_a THEN
        level := level + 0.8;
    END_IF;
    IF valve_b THEN
        level := level + 1.0;
    END_IF;
    IF drain THEN
        level := level - 1.2;
        total_drained := total_drained + 1.2;
    END_IF;
    IF heater THEN
        temperature := temperature + 0.4;
    END_IF;
    IF mixer AND NOT heater THEN
        temperature := temperature - 0.08;
    END_IF;

    (* Clamp *)
    IF level > 100.0 THEN
        level := 100.0;
    END_IF;
    IF level < 0.0 THEN
        level := 0.0;
    END_IF;
    IF temperature < 0.0 THEN
        temperature := 0.0;
    END_IF;
    IF temperature > 120.0 THEN
        temperature := 120.0;
    END_IF;

END_PROGRAM
