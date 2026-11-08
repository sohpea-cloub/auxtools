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

// Phase 5: procs, overrides and var defaults the test adds or changes

/datum/animal
	var/sound = "..."
	var/legs = 4

/datum/animal/proc/speak()
	return sound

/datum/animal/dog
	sound = "woof"

/datum/animal/dog/puppy

/datum/animal/dog/puppy/speak()
	return "[..()]?"

/datum/animal/cat
	sound = "meow"

/datum/animal/bird
	legs = 2

var/datum/animal/dog/old_dog = new
var/datum/animal/dog/puppy/old_puppy = new
var/datum/animal/dog/legs_written = new

/proc/write_legs()
	legs_written.legs = 9

/proc/new_dog_legs()
	var/datum/animal/dog/d = new
	return d.legs

/proc/new_bird_legs()
	var/datum/animal/bird/b = new
	return b.legs

/proc/uses_helper()
	return 0

/proc/greet(n)
	return "hi [n]"

/proc/greet_named()
	return "unchanged"

/proc/slow()
	sleep(1)
	return 1

/proc/caller_sees()
	var/x = slow()
	return isnull(x)
