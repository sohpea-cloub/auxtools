/proc/auxtools_stack_trace(msg)
	CRASH(msg)

/proc/auxtools_expr_stub()
	CRASH("auxtools not loaded")

// The debug server hooks this one, so new code for it never runs
/proc/enable_debugging(mode, port)
	CRASH("auxtools not loaded")

/world/New()
	var/debug_dll = world.GetConfig("env", "AUXTOOLS_DEBUG_DLL")
	var/init_result = call_ext(debug_dll, "auxtools_init")()
	world.log << "init_result = [init_result]"
	ASSERT(init_result == "SUCCESS")

	// No port given, so it comes from AUXTOOLS_DEBUG_PORT
	enable_debugging("BACKGROUND")

	spawn()
		sleeper()
	spawn()
		main_loop(debug_dll)
	. = ..()

var/reboot_requested = FALSE

/proc/request_reboot()
	reboot_requested = TRUE

// The debug server only answers while DM code runs
/proc/main_loop(debug_dll)
	while (!reboot_requested)
		tick()
		sleep(1)

	// Shutting down has to happen out here. The test asks for the reboot
	// through the debug server, which can't free itself.
	call_ext(debug_dll, "auxtools_shutdown")()
	world.Reboot()
