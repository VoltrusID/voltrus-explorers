PROGRAM MotorControl
VAR
    (* Inputs *)
    start_btn : BOOL := FALSE;
    stop_btn : BOOL := FALSE;
    emergency_stop : BOOL := FALSE;
    overload_trip : BOOL := FALSE;

    (* Outputs *)
    motor_running : BOOL := FALSE;
    fault : BOOL := FALSE;
    speed : INT := 0;
    fault_code : INT := 0;

    (* Parameters *)
    target_speed : INT := 1000;
    ramp_rate : INT := 10;
    decel_rate : INT := 25;

    (* Runtime *)
    run_time : DINT := 0;
    start_delay : INT := 0;
END_VAR

    run_time := run_time + 1;

    (* Safety: check overload *)
    IF overload_trip THEN
        fault := TRUE;
        fault_code := 1;
        motor_running := FALSE;
    END_IF;

    (* Safety: emergency stop *)
    IF emergency_stop THEN
        fault := TRUE;
        fault_code := 2;
        motor_running := FALSE;
    END_IF;

    (* Start logic: latch on *)
    IF start_btn AND NOT stop_btn AND NOT fault THEN
        IF (NOT motor_running) AND (speed = 0) THEN
            start_delay := start_delay + 1;
            IF start_delay > 10 THEN
                motor_running := TRUE;
                start_delay := 0;
            END_IF;
        ELSE
            motor_running := TRUE;
            start_delay := 0;
        END_IF;
    END_IF;

    (* Stop logic *)
    IF stop_btn THEN
        motor_running := FALSE;
        start_delay := 0;
    END_IF;

    (* Fault reset: hold stop for 2 seconds *)
    IF stop_btn AND fault THEN
        start_delay := start_delay + 1;
        IF start_delay > 20 THEN
            fault := FALSE;
            fault_code := 0;
            start_delay := 0;
        END_IF;
    END_IF;

    (* Speed ramping *)
    IF motor_running THEN
        IF speed < target_speed THEN
            speed := speed + ramp_rate;
            IF speed > target_speed THEN
                speed := target_speed;
            END_IF;
        END_IF;
    ELSE
        IF speed > 0 THEN
            speed := speed - decel_rate;
            IF speed < 0 THEN
                speed := 0;
            END_IF;
        END_IF;
    END_IF;

END_PROGRAM
