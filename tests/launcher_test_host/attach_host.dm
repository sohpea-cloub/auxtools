// Started by auxtools' launcher without debugging, then attached to

var/rounds = 0

/world/New()
	..()
	spawn(1)
		run_until_stopped()

/proc/run_until_stopped()
	var/said = 0
	while (!fexists("stop"))
		world.log << "attach test: [value()]"
		rounds++
		said++
		// A runtime error when the test asks for one
		if (fexists("fail"))
			fdel("fail")
			spawn(0)
				fail()
		sleep(5)
	world.log << "attach test: done"
	del(world)

/proc/value()
	return 1

/proc/fail()
	var/list/L = null
	return L[1]
