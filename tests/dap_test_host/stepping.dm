/proc/leaf(x)
	var/y = x + 1 // @leaf_1
	return y * 2 // @leaf_2

/proc/middle(a)
	var/r = leaf(a) // @middle_1
	r += leaf(r) // @middle_2
	return r // @middle_3

/proc/sc_middle()
	var/got = middle(3) // @sc_middle_1
	return got // @sc_middle_2

/proc/sleeper(n)
	var/before = n // @sleeper_1
	sleep(2) // @sleeper_2
	var/after = before + 1 // @sleeper_3
	return after // @sleeper_4

/proc/sc_sleeper()
	sleeper(7)

/proc/spawner()
	var/count = 0 // @spawner_1
	spawn(1) // @spawner_2
		count += 10 // @spawner_body
	count += 1 // @spawner_3
	return count // @spawner_4

/proc/sc_spawner()
	spawner()
	sleep(3)

/proc/recurse(depth)
	if (depth <= 0) // @recurse_1
		return leaf(0) // @recurse_leaf
	var/below = recurse(depth - 1) // @recurse_2
	return below + 1 // @recurse_3

/proc/sc_recurse()
	recurse(3)

/proc/sc_deep()
	recurse(120)

/datum/thing
	var/name = "thing"
	var/n = 1
	var/list/items
	var/datum/thing/other
	var/tmp/scratch = 3
	var/static/shared = 9
	var/const/K = 4

/datum/thing/New(called)
	if (called)
		name = called

/datum/thing/proc/bump(by)
	n += by // @bump_1
	return n

/datum/thing/sub/bump(by)
	var/got = ..(by * 2) // @sub_bump_1
	return got + 1

/datum/thing/twice/bump(by)
	return ..(by) // @twice_first

/datum/thing/twice/bump(by)
	return ..(by) + 100 // @twice_second

/proc/sc_twice()
	var/datum/thing/twice/t = new("T")
	return t.bump(1)

/proc/sc_parent()
	var/datum/thing/sub/s = new("S")
	var/r = s.bump(2) // @sc_parent_1
	return r

/datum/made
	var/list/stuff = list(1, 2)
	var/size = 0

/datum/made/New(n)
	size = n // @made_new

/proc/sc_new()
	var/datum/made/m = new(5) // @sc_new_1
	return m.size // @sc_new_2

/proc/sc_call()
	var/r = call(/proc/leaf)(1) // @sc_call_1
	return r

/proc/sc_plain()
	var/a = 1 // @plain_1
	var/b = a + 1 // @plain_2
	return a + b // @plain_3

/proc/quiet_failure()
	var/list/L = null
	return L[1]

/proc/sc_flow()
	var/total = 0 // @flow_1
	for (var/i in 1 to 2) // @flow_for
		total += i // @flow_body
	if (total > 5) // @flow_if
		total = 0
	else
		total += 1 // @flow_else
	switch (total) // @flow_switch
		if (4)
			total = 40 // @flow_case
		else
			total = -1
	try // @flow_try_start
		total += quiet_failure() // @flow_try
	catch (var/exception/e)
		total += length(e.name) ? 1 : 0 // @flow_catch
	return total // @flow_return

/mob/player
	var/hp = 10

/mob/player/verb/shout(msg as text)
	return "shout [msg]" // @shout

/proc/sc_verb()
	var/mob/player/M = new(locate(1, 1, 1))
	M.shout("x")
	del(M)

var/stop_busy = FALSE

/proc/sc_busy()
	var/started = world.timeofday
	var/passes = 0
	stop_busy = FALSE
	world.log << "dap: busy"
	while (!stop_busy && abs(world.timeofday - started) < 50)
		passes++
	return passes

var/many = 50

/proc/sc_many()
	for (var/i in 1 to many)
		spawn(0)
			sleep(600) // @many_sleep
	sleep(1)
	leaf(1)

/proc/nap(ticks)
	var/napped = ticks
	sleep(ticks) // @nap_2
	return napped // @nap_3

/proc/sc_nap()
	spawn(0)
		nap(10)
	sleep(1)
	leaf(1)

/datum/dozer
	var/n = 0

/datum/dozer/proc/doze()
	set waitfor = FALSE
	var/mine = n
	sleep(600)
	return mine

var/list/dozers

/proc/end_dozer(datum/dozer/d)
	del(d)

/proc/sc_dozers()
	dozers = list()
	for (var/i in 1 to 3)
		var/datum/dozer/d = new
		d.n = i
		dozers += d
		d.doze()
	sleep(1)
	leaf(1)
	for (var/datum/dozer/d in dozers)
		del(d)
	dozers = null

#define NOTHING_TO_RUN
/proc/nothing_to_run()
	NOTHING_TO_RUN // @nothing_macro
	var/n // @nothing_var
	set background = TRUE // @nothing_set
	n = 1 // @nothing_code
	
	var/text = "[n]"
	var/size = length(text) // @read_ahead_next
	return n * size

/proc/sc_nothing_to_run()
	nothing_to_run()

/proc/spawns_now()
	var/n = 1
	spawn (-1) // @spawn_now_spawn
		n = 2 // @spawn_now_body
	n = 3 // @spawn_now_2
	return n

/proc/sc_spawn_now()
	spawns_now()

/proc/spawns_sleeping()
	var/n = 1
	spawn (-1) // @spawn_sleep_spawn
		sleep(2)
		n = 2

/proc/sc_spawn_sleep()
	spawns_sleeping() // @spawn_sleep_call
	sleep(5)

var/pair_held = FALSE

/proc/pair(first, second)
	while (pair_held)
		sleep(1)
	return first - second // @pair_return

/proc/sc_pair_sleeps()
	pair_held = TRUE
	spawn(0)
		world.log << "dap: the old pair returned [pair(1, 2)]"

/proc/sc_pair()
	pair_held = FALSE
	world.log << "dap: the new pair returned [pair(1, 2)]"

var/lone_held = FALSE

/proc/lone(value)
	while (lone_held)
		sleep(1)
	return value // @lone_return

/proc/sc_lone_sleeps()
	lone_held = TRUE
	spawn(0)
		world.log << "dap: the old lone returned [lone(1)]"

/proc/sc_lone()
	lone_held = FALSE
	world.log << "dap: the new lone returned [lone(2)]"

/proc/sc_swap()
	var/n = 1
	n += 2 // @swap_add
	n *= 3 // @swap_mul
	return n

/proc/sc_noted()
	var/n = 1
	// A comment above a statement
	n += 1 // @noted_add
	return n
