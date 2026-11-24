var/sleeper_state = "not started"
var/sleeper_release = FALSE
var/runtime_armed = FALSE
var/breakpoint_armed = FALSE

#define PARAM_TWINS(first, first_param, second, second_param) /proc/first(first_param) { twin_waiting += 1; while (twin_hold) sleep(1); twin_runs += first_param; } /proc/second(second_param) { twin_waiting += 1; while (twin_hold) sleep(1); twin_runs += second_param; }
PARAM_TWINS(twin_e, e_param, twin_f, f_param)

var/big_count = 0
#define BIG1 big_count += 1;
#define BIG4 BIG1 BIG1 BIG1 BIG1
#define BIG16 BIG4 BIG4 BIG4 BIG4
#define BIG64 BIG16 BIG16 BIG16 BIG16
#define BIG256 BIG64 BIG64 BIG64 BIG64
#define BIG1K BIG256 BIG256 BIG256 BIG256
#define BIG4K BIG1K BIG1K BIG1K BIG1K
#define BIG_PROC(name, param) /proc/name(param) { BIG4K BIG4K BIG4K BIG256 BIG256 BIG256 BIG16 BIG16 BIG16 BIG1 BIG1 }

/proc/answer()
	return 1

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

#define TWINS(first, second) /proc/first() { twin_waiting += 1; while (twin_hold) sleep(1); twin_runs += 1; } /proc/second() { twin_waiting += 1; while (twin_hold) sleep(1); twin_runs += 1; }
var/twin_runs = 0
var/twin_waiting = 0
var/twin_hold = FALSE
var/twin_armed = null
TWINS(twin_a, twin_b)
#define NAMED_TWINS(first, first_local, second, second_local) /proc/first() { var/first_local = 1; twin_waiting += 1; while (twin_hold) sleep(1); twin_runs += first_local; } /proc/second() { var/second_local = 1; twin_waiting += 1; while (twin_hold) sleep(1); twin_runs += second_local; }
NAMED_TWINS(twin_c, c_local, twin_d, d_local)

var/caught_armed = FALSE
var/caught_by_tick = null
var/caught_done = 0

/proc/caught_target()
	var/y = 1
	y += 1
	caught_done = y

/proc/tick()
	if (caught_armed)
		caught_armed = FALSE
		try
			caught_target()
		catch (var/exception/e)
			caught_by_tick = e.name
	if (breakpoint_armed)
		breakpoint_armed = FALSE
		breakpoint_target()
	if (twin_armed)
		var/twin = twin_armed
		twin_armed = null
		spawn()
			call(twin)(1)
	if (runtime_armed)
		runtime_armed = FALSE
		runtime_target()

/datum/animal
	var/sound = "..."
	var/legs = 4
	var/spare = 1

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

/proc/in_lists(mob/m in world, o in global.vars)
	return 1

/datum/bag
	var/list/items = list("a")

/datum/bag/big

/datum/bag/fixed
	items = list("z")

var/datum/bag/old_bag = new

/proc/new_items(type)
	var/datum/bag/b = new type
	return b.items ? jointext(b.items, ",") : "none"

/obj/crate
	name = "crate"
	var/capacity = 4
	var/list/contents_log = list("empty")

/obj/crate/proc/describe_crate()
	return "[name] holds [capacity]"

/proc/crate_report(type)
	var/obj/crate/c = new type
	return "[c.name]|[c.desc]|[c.icon_state]|[c.density]|[c.capacity]|[c.describe_crate()]|[c.contents_log[1]]|[c.text]"

/datum/napper/proc/nap()
	set waitfor = FALSE
	sleep(1)
	return 1

/datum/napper/cat/nap()
	sleep(1)
	return 2

/proc/nap_skipped(type)
	var/datum/napper/n = new type
	return isnull(n.nap())

/mob/helper/verb/greet()
	set name = "Greet"
	set category = "Helpers"
	return "hi"

/mob/helper/small

var/mob/helper/small/old_helper = new

/proc/verb_report(mob/m)
	var/list/out = list()
	for (var/v in m.verbs)
		out += "[v:name]:[v:category]"
	return jointext(out, ",")

/proc/helper_greets(mob/helper/m)
	return m.greet()

/proc/first_def()
	return "first"

/datum/early/speak()
	return "override"

/datum/early/proc/speak()
	return "declaration, then [..()]"

/datum/early_apart/proc/speak()
	return "declaration apart, then [..()]"

/datum/late/proc/speak()
	return "late declaration, then [..()]"

/proc/early_global()
	return "early declared"

/proc/calls_early_global()
	return early_global()

/datum/twice/proc/speak()
	return "declaration"

/datum/twice/speak()
	return "first override"

/datum/twice/speak()
	return "second override"

/datum/inserted/proc/speak()
	return "declaration"

/datum/inserted/speak()
	return "old first override"

/datum/kin/proc/speak()
	return "declaration"

/datum/kin/middle/speak()
	return "parent override"

/datum/kin/middle/child

/datum/lone/proc/speak()
	return "declaration"

/datum/lone/speak()
	return "lone override"

/obj/picker

/datum/handled/proc/respond()
	return "declaration"

/datum/handled/respond()
	return "override"

/datum/holder

/datum/gone/proc/speak()
	return "gone declaration"

/datum/last/proc/speak()
	return "declaration"

/datum/last/speak()
	return "first override"

/datum/last/speak()
	return "last override"
