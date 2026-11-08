// The procs the test changes. It edits this file by replacing exact text, so
// mind the tests when changing anything here.

var/sleeper_state = "not started"
var/sleeper_release = FALSE
var/runtime_armed = FALSE
var/breakpoint_armed = FALSE

/proc/answer()
	return 1

// Started when the world starts, and waits until the test lets it finish
/proc/sleeper()
	sleeper_state = "sleeping"
	while (!sleeper_release)
		sleep(1)
	sleeper_state = "old code finished"

/proc/release_sleeper()
	sleeper_release = TRUE

/proc/start_sleeper()
	spawn()
		sleeper()

/proc/factorial(n)
	if (n <= 1)
		return 1
	return n * factorial(n - 1)

/proc/few_locals()
	var/a = 1
	return a

// Gives the world a locals table with room for six
/proc/many_locals()
	var/a = 1
	var/b = 2
	var/c = 3
	var/d = 4
	var/e = 5
	var/f = 6
	return a + b + c + d + e + f

/proc/counter()
	var/static/count = 0
	count++
	return count

/proc/arm_runtime()
	runtime_armed = TRUE

/proc/runtime_target()
	return 1

/proc/arm_breakpoint()
	breakpoint_armed = TRUE

/proc/breakpoint_target()
	var/x = 1
	x += 1
	return x

/proc/tick()
	if (breakpoint_armed)
		breakpoint_armed = FALSE
		breakpoint_target()
	if (runtime_armed)
		runtime_armed = FALSE
		runtime_target()
