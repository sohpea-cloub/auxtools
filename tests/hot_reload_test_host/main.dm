// Started with the debug library loaded into it, which adds the procs it
// needs. It hooks this one when it's the world's own, so new code for it never
// runs.
/proc/auxtools_launched_world_del()
	CRASH("auxtools not loaded")

/world/New()
	spawn()
		sleeper()
	spawn()
		main_loop()
	. = ..()

var/reboot_requested = FALSE

/proc/request_reboot()
	reboot_requested = TRUE

// The debug server only answers while DM code runs
/proc/main_loop()
	while (!reboot_requested)
		tick()
		sleep(1)

	// Rebooting has to happen out here. The test asks for it through the
	// debug server, which can't shut itself down there.
	world.Reboot()
