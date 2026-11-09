// Started by auxtools' launcher without debugging, then attached to

/world/New()
	..()
	spawn(1)
		run_until_stopped()

/proc/run_until_stopped()
	while (!fexists("stop"))
		world.log << "attach test: [value()]"
		sleep(5)
	world.log << "attach test: done"
	del(world)

/proc/value()
	return 1
