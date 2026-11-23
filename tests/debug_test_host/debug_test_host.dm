var/runtimes_left = 0
var/reboot_requested = FALSE

/proc/breakpoint_target()
	return 2

/proc/tick_marker()
	return 1

#if DM_VERSION >= 516
/proc/alist_sample()
	return alist("a" = 1, 7 = "seven", "n" = null)

/proc/vector_sample()
	return vector(1, 2, 3)

/proc/vector2_sample()
	return vector(4, 5)

/proc/pixloc_sample()
	return pixloc(1, 1, 1)
#endif

/proc/profiled_loop(n)
	var/s = 0
	for (var/i in 1 to n)
		s += i
	return s

/proc/profiled_call()
	return profiled_loop(1000)

#define PROFILED_TWINS(first, second) /proc/first(n) { var/s = 0; for (var/i in 1 to n) s += i; return s } /proc/second(n) { var/s = 0; for (var/i in 1 to n) s += i; return s }
PROFILED_TWINS(profiled_twin_a, profiled_twin_b)

/proc/profiled_countdown(n)
	while (n > 0)
		n--
	return n

/proc/watched_steps()
	var/steps = 1
	steps = 2
	. = steps

/proc/throw_caught()
	try
		throw "thrown and caught"
	catch
		return 1

/proc/start_runtimes()
	runtimes_left = 20

/proc/make_runtime()
	var/list/nothing
	return nothing.len

/proc/request_reboot()
	reboot_requested = TRUE

/world
	fps = 100

/proc/main_loop()
#if DM_VERSION >= 516
	var/callee/this_proc = callee
#endif
	while (!reboot_requested)
		breakpoint_target()
		tick_marker()
		if (runtimes_left > 0)
			runtimes_left--
			spawn()
				make_runtime()
		sleep(world.tick_lag)

	world.Reboot()

/world/New()
	maxx = 5
	maxy = 5
	maxz = 1

	spawn()
		main_loop()
	. = ..()

/mob/helper/verb/greet(who as text)
	set name = "Greet Someone"
	return "hi [who]"

var/mob/helper/helper = new

var/overrun_global = 2

/proc/overrun_args(x1 = null, x2, x3)

/datum/overrun
	var/list/items = list(1, 2)

/datum/overrun/proc/stored(x1, x2 = "")
	var/loops = 0
	var/n1 = 256
	var/s1 = "3.5"
	var/list/L = list(1, "  pad  ")
	var/v
	src.items["b"] ||= overrun_args((args.Copy() + null), text2path("/datum/nope"), src.items["k"])
	var/list/l45 = (L - (min(max(overrun_global, -16777216), 16777215) << 2))

/datum/overrun/proc/calls(a, b = 2, c)
	var/datum/overrun/o = new /datum/overrun
	var/datum/p = new /datum
	var/v
	if (isloc((v || 1)))
	else
		o.stored(findlasttextEx({"multi "quoted" text"}, "a\tb"), "[p]\t[null])")

/proc/overrun()
	var/datum/overrun/o = new
	o.calls(113, 227, "c113")
	return 7
