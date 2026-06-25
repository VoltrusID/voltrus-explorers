PROGRAM TrafficLight
VAR
    state : INT := 0;
    timer : INT := 0;
    red : BOOL := TRUE;
    yellow : BOOL := FALSE;
    green : BOOL := FALSE;
    flash : BOOL := FALSE;
    flash_counter : INT := 0;
    pedestrian_waiting : BOOL := FALSE;
END_VAR

    timer := timer + 1;
    flash_counter := flash_counter + 1;
    flash := (flash_counter MOD 20) > 10;

    IF state = 0 THEN
        (* Red - stop traffic *)
        red := TRUE;
        yellow := FALSE;
        green := FALSE;
        IF timer > 100 THEN
            state := 1;
            timer := 0;
        END_IF;

    ELSIF state = 1 THEN
        (* Red + Yellow - prepare to go *)
        red := TRUE;
        yellow := TRUE;
        green := FALSE;
        IF timer > 30 THEN
            state := 2;
            timer := 0;
        END_IF;

    ELSIF state = 2 THEN
        (* Green - go *)
        red := FALSE;
        yellow := FALSE;
        green := TRUE;
        IF timer > 100 THEN
            state := 3;
            timer := 0;
        END_IF;

    ELSIF state = 3 THEN
        (* Yellow - warning *)
        red := FALSE;
        yellow := TRUE;
        green := FALSE;
        IF timer > 40 THEN
            state := 0;
            timer := 0;
        END_IF;
    END_IF;

END_PROGRAM
