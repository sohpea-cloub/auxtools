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

// A type's const is compiled into the code that uses it
/datum/limits
	var/const/MAX = 3

/datum/limits/proc/max_doubled()
	return MAX * 2

var/datum/limits/kept_limits = new

// A var declared with another type later
/datum/limits/var/list/things = null

/datum/limits/proc/thing_count()
	return length(things)

// A proc whose `set name` changes, which call() by name follows
/datum/limits/proc/label()
	set name = "Label"
	return "labelled"
