var/rounds = 0

#define DOUBLE_IT(x) ((x) * 2)
#define ANSWER 42
#define GONE 1
#undef GONE

var/datum/held/held = new

/datum/held
	var/count = 3

/datum/cached
	var/static/list/cache = list()

/datum/held/proc/double(n)
	return n * 2

/proc/slow_double(n, wait = 2)
	sleep(wait)
	return n * 2

/proc/slow_fail()
	sleep(1)
	return fail_deeper(5)

/world
	fps = 100

/world/New()
	..()
	spawn(1)
		run_until_stopped()
	doze()

/proc/doze()
	set waitfor = FALSE
	sleep(36000)

/world/Del()
	world.log << "attach test: world/Del"
	if (fexists("slow_del"))
		sleep(3)
	if (fexists("refuse_del"))
		return
	if (fexists("fail_del"))
		CRASH("attach test: world/Del failed")
	..()

/world/Reboot()
	if (fexists("slow_reboot"))
		sleep(3)
	if (fexists("refuse_reboot"))
		world.log << "attach test: reboot refused"
		return
	return ..()

/proc/run_until_stopped()
	var/said = 0
	while (!fexists("stop"))
		world.log << "attach test: [value()]"
		tally()
		rounds++
		said++
		if (rounds % 5 == 0)
			if (fexists("fail"))
				fdel("fail")
				spawn(0)
					fail()
			if (fexists("failing"))
				spawn(0)
					fail()
			if (fexists("reboot"))
				fdel("reboot")
				world.Reboot()
			if (fexists("call"))
				fdel("call")
				spawn(0)
					fail_in(1)
		sleep(1)
	world.log << "attach test: done"
	del(world)

/proc/value()
	return 1

/proc/tally()
	tick(1)
	if (rounds % 2 == 0)
		tick(2)

/proc/tick(n)
	return n

/proc/fail()
	var/list/L = null
	return L[1]

/obj/macro_named
	name = "\improper AI Chamber"

/datum/inits
	var/list/given = list(1)
/proc/two_loops()
	var/total = 0
	for(var/i = 1, i <= 3, i++)
		total += i // first loop
	for(var/i in list(10, 20))
		total += i // second loop
	return total

/proc/fail_in(n)
	var/got = fail_deeper(n)
	return got

/proc/fail_deeper(n)
	var/list/L = list()
	return L[n]

/proc/caught_inside()
	try
		fail()
	catch
		return "caught"

/proc/client_type()
	return /client

/datum/twice/proc/act()
	return "first"

/datum/twice/act()
	. = ..()
	return "second"

/datum/twice/more

/proc/twice_ref()
	return /datum/twice/more::act()

/proc/act()
	return "global"

/atom/proc/atom_act()
	return "atom"

/proc/last_in_file()
	return "last [rounds]"
