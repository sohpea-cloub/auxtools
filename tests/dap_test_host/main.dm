var/scenario
var/ticks = 0
var/idle_ticks = 0
var/global_number = 5
var/list/global_list = list(1, "two", 3)
var/early_global = early_value()

/proc/early_value()
	if (fexists("fail_early"))
		var/list/L = null
		return L[1] // @early_failure
	return 5 // @early_value

/world
	fps = 100

/world/New()
	maxx = 3
	maxy = 3
	maxz = 1
	..()
	world.log << "dap: world/New" // @world_new
	spawn(1)
		main_loop()

/world/Del()
	world.log << "dap: world/Del"
	..()

/proc/main_loop()
	while (TRUE)
		ticks++
		if (scenario)
			var/name = scenario
			scenario = null
			world.log << "dap: running [name]"
			spawn(0)
				call("/proc/sc_[name]")()
				world.log << "dap: finished [name]"
		if (idle_ticks)
			var/idle = idle_ticks
			idle_ticks = 0
			world.log << "dap: idling"
			sleep(idle)
			world.log << "dap: idle over"
		sleep(world.tick_lag)

/proc/sc_quit()
	world.log << "dap: quitting"
	del(world)

/proc/sc_reboot()
	world.log << "dap: rebooting"
	world.Reboot()

/proc/icon_built()
	return 'reload_built.dmi'

/proc/icon_built_2()
	return 'reload_built_2.dmi'

/proc/icon_reloaded()
	return null

/proc/sc_stderr()
	world.log << "dap: to the log"
