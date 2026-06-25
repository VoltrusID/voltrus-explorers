PROGRAM TankLevel
VAR
    (* Process variable *)
    level : REAL := 25.0;
    temperature : REAL := 20.0;

    (* Setpoints *)
    level_setpoint : REAL := 50.0;
    temp_setpoint : REAL := 65.0;
    deadband : REAL := 3.0;

    (* Outputs *)
    inlet_valve : BOOL := FALSE;
    outlet_valve : BOOL := FALSE;
    heater : BOOL := FALSE;
    cooling_valve : BOOL := FALSE;

    (* Alarms *)
    high_level_alarm : BOOL := FALSE;
    low_level_alarm : BOOL := FALSE;
    high_temp_alarm : BOOL := FALSE;

    (* Simulation parameters *)
    inlet_flow : REAL := 1.0;
    outlet_flow : REAL := 0.6;
    heat_rate : REAL := 0.3;
    cool_rate : REAL := 0.15;
    ambient_loss : REAL := 0.05;

    (* Statistics *)
    cycles : DINT := 0;
    total_inlet_time : DINT := 0;
    total_outlet_time : DINT := 0;
END_VAR

    cycles := cycles + 1;

    (* Level control - bang-bang with deadband *)
    IF level < level_setpoint - deadband THEN
        inlet_valve := TRUE;
        outlet_valve := FALSE;
    ELSIF level > level_setpoint + deadband THEN
        inlet_valve := FALSE;
        outlet_valve := TRUE;
    END_IF;

    (* Temperature control *)
    IF temperature < temp_setpoint - deadband THEN
        heater := TRUE;
        cooling_valve := FALSE;
    ELSIF temperature > temp_setpoint + deadband THEN
        heater := FALSE;
        cooling_valve := TRUE;
    END_IF;

    (* Alarms *)
    high_level_alarm := level > 90.0;
    low_level_alarm := level < 10.0;
    high_temp_alarm := temperature > 85.0;

    (* Emergency shutdown *)
    IF high_level_alarm THEN
        inlet_valve := FALSE;
    END_IF;
    IF high_temp_alarm THEN
        heater := FALSE;
    END_IF;

    (* Simulate process dynamics *)
    IF inlet_valve THEN
        level := level + inlet_flow;
        total_inlet_time := total_inlet_time + 1;
    END_IF;
    IF outlet_valve THEN
        level := level - outlet_flow;
        total_outlet_time := total_outlet_time + 1;
    END_IF;
    IF heater THEN
        temperature := temperature + heat_rate;
    END_IF;
    IF cooling_valve THEN
        temperature := temperature - cool_rate;
    END_IF;

    (* Ambient heat loss *)
    temperature := temperature - ambient_loss;

    (* Clamp values *)
    IF level > 100.0 THEN
        level := 100.0;
    END_IF;
    IF level < 0.0 THEN
        level := 0.0;
    END_IF;
    IF temperature < 0.0 THEN
        temperature := 0.0;
    END_IF;
    IF temperature > 100.0 THEN
        temperature := 100.0;
    END_IF;

END_PROGRAM
