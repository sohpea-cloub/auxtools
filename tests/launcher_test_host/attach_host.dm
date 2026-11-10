// Started by auxtools' launcher without debugging, then attached to

var/rounds = 0

/world/New()
	..()
	spawn(1)
		run_until_stopped()

// A world whose shutdown code doesn't shut it down, when the test asks for one
/world/Del()
	world.log << "attach test: world/Del"
	if (fexists("refuse_del"))
		return
	..()

/proc/run_until_stopped()
	var/said = 0
	while (!fexists("stop"))
		world.log << "attach test: [value()]"
		tally()
		rounds++
		said++
		// A runtime error when the test asks for one
		if (fexists("fail"))
			fdel("fail")
			spawn(0)
				fail()
		// A reboot when the test asks for one
		if (fexists("reboot"))
			fdel("reboot")
			world.Reboot()
		sleep(5)
	world.log << "attach test: done"
	del(world)

/proc/value()
	return 1

// Calls tick() from two places: every round, and every other round, for
// counting hits by caller
/proc/tally()
	tick(1)
	if (rounds % 2 == 0)
		tick(2)

/proc/tick(n)
	return n

/proc/fail()
	var/list/L = null
	return L[1]
