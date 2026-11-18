// The world `auxtools optimise --globals` is tested on. Two procs can be
// global procs; each of the others breaks one rule. With GLOBAL the two are
// global procs in source. The world writes what calling each gives to
// result.txt.
#define OUT(X) text2file(X, "result.txt")
#ifdef GLOBAL
#define HELPER(X) /proc/##X
#else
#define HELPER(X) /datum/a/proc/##X
#endif

/datum/a
	var/datum/a/other
	var/count = 10

// made global: declared once, no src, cannot sleep, called on src alone
HELPER(pure_helper)(x, y)
	var/list/both = list(x, y)
	both.Add(3)
	return both.len + abs(x) + global_proc(y)
HELPER(fails_inside)(list/short)
	return short[5]

// kept: uses src
/datum/a/proc/reads_src(x)
	return x + count
// kept: overridden
/datum/a/proc/overridden(x)
	return x
/datum/a/sub/overridden(x)
	return ..() + 1
// kept: called on another object, which may be null or of another type
/datum/a/proc/on_another(x)
	return x * 2
// kept: can sleep, itself or through what it calls
/datum/a/proc/sleeps(x)
	sleep(3)
	OUT("sleeps resumed [x]")
	return x
/datum/a/proc/calls_sleeper(x)
	. = global_sleeper(x)
	OUT("calls_sleeper resumed [x]")
/datum/a/proc/spawns(x)
	spawn(1)
		OUT("spawned [x]")
	return x
/datum/a/proc/in_background(x)
	set background = 1
	return x
/datum/a/proc/calls_by_path(x)
	return call(/proc/global_proc)(x)
/datum/a/proc/makes_new(x)
	var/datum/sleepy/made = new
	return made.slept + x
// kept: call() can find it by name, and its path is a value
/datum/a/proc/by_name(x)
	return x + 3
/datum/a/proc/as_value(x)
	return x + 4
// kept: another type has the identifier
/datum/a/proc/same_identifier(x)
	return x + 5
/datum/unrelated/proc/same_identifier(x)
	return x + 6
// kept: a verb
/datum/a/verb/a_verb(x as num)
	return x + 7

/datum/sleepy
	var/slept = 0
/datum/sleepy/New()
	sleep(1)
	slept = 1

/proc/global_proc(y)
	return y
/proc/global_sleeper(x)
	sleep(3)
	return x

/datum/a/proc/calls_pure(x)
	. = pure_helper(x, 2)
	pure_helper(x, 3)
	. += count
	if(x)
		. += pure_helper(arglist(list(1, 2)))
	. += pure_helper(y = 1, x = 2) + count
	for(var/i in 1 to 3)
		. += pure_helper(i, i)
	switch(x)
		if(0)
			. += pure_helper(5, 5)
		if(1)
			return . + pure_helper(6, 6) + count
	. += count
/datum/a/sub/proc/from_below(x)
	return pure_helper(x, x) + count + pure_helper(1, 1)
/datum/a/proc/calls_kept(x)
	var/as_value = /datum/a/proc/as_value
	return "[reads_src(x)] [overridden(x)] [other.on_another(x)] [on_another(x)] [in_background(x)] [calls_by_path(x)] [call(src, "by_name")(x)] [by_name(x)] [call(src, as_value)(x)] [as_value(x)] [same_identifier(x)] [a_verb(x)]"
/datum/a/proc/calls_sleepers(x)
	OUT("sleepers: [sleeps(x)] [calls_sleeper(x)] [spawns(x)] [makes_new(x)]")
/datum/a/proc/error_inside()
	try
		fails_inside(list(1))
	catch(var/exception/e)
		for(var/line in splittext("[e.desc]", "\n"))
			if(findtext(line, "src:"))
				return "[e.name];[line]"

/proc/results()
	var/datum/a/sub/thing = new
	thing.other = thing
	OUT("made global: [thing.calls_pure(1)] [thing.calls_pure(0)] [thing.from_below(4)]")
	OUT("kept: [thing.calls_kept(1)]")
	OUT("error: [thing.error_inside()]")
	thing.calls_sleepers(2)
	// Procs sleeping on an object end when it is deleted
	var/datum/a/gone = new
	gone.other = gone
	spawn(0)
		gone.calls_sleepers(3)
	sleep(1)
	del(gone)
	sleep(15)

/world/New()
	..()
	fdel("result.txt")
	try
		results()
	catch(var/exception/e)
		OUT("runtime error: [e.name]")
	OUT("done")
	shutdown()
