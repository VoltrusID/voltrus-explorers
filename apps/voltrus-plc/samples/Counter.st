PROGRAM Counter
VAR
    counter : INT := 0;
    max_count : INT := 100;
    done : BOOL := FALSE;
    cycles : DINT := 0;
    direction : INT := 1;
END_VAR

    cycles := cycles + 1;

    counter := counter + direction;

    IF counter >= max_count THEN
        direction := -1;
        done := TRUE;
    ELSIF counter <= 0 THEN
        direction := 1;
        done := FALSE;
    END_IF;

END_PROGRAM
