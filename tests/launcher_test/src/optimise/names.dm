// The world `auxtools optimise` is tested on: each identifier is one rule of
// the names pass. The world writes what every kind of call gives to
// result.txt. With NAMED, each proc the pass renames is named in source.
#define OUT(X) text2file(X, "result.txt")
#ifdef NAMED
#define NAME_IT(X) set name = X
#else
#define NAME_IT(X)
#endif

/datum/a
	var/datum/a/other
// an override compiled before the declaration it overrides
/datum/a/b/plain_proc(x)
	return "b" + ..()
/datum/a/b/c/plain_proc(x)
	return "c" + ..()
/datum/a/proc/plain_proc(x)
	NAME_IT("plain_proc")
	return "a[x]"
// the same identifier on an unrelated type
/datum/other/proc/plain_proc(x)
	NAME_IT("plain_proc")
	return "other[x]"
/datum/a/proc/calls_itself(x)
	NAME_IT("calls_itself")
	return plain_proc(x) + src.plain_proc(x) + other.plain_proc(x)
/datum/a/proc/does_not_wait(x)
	NAME_IT("does_not_wait")
	set waitfor = 0
	return "nowait[x]"
/datum/a/proc/in_background(x)
	NAME_IT("in_background")
	set background = 1
	return "background[x]"
/datum/a/proc/will_fail()
	NAME_IT("will_fail")
	var/list/nothing = null
	return nothing[1]

// kept: "has text" is text here, and call() finds the proc by it
/datum/a/proc/has_text(x)
	return "text[x]"
// kept: verbs and procs with verb settings
/datum/a/verb/a_verb(x as num)
	return "verb[x]"
/datum/a/proc/with_category(x)
	set category = "Things"
	return "category[x]"
/datum/a/proc/with_desc(x)
	set desc = "Described"
	return "desc[x]"
/datum/a/proc/is_hidden(x)
	set hidden = 1
	return "hidden[x]"
// kept: named by its author
/datum/a/proc/custom_named(x)
	set name = "Custom"
	return "custom[x]"
// kept: its path is a value
/datum/a/proc/used_as_value(x)
	return "value[x]"
// kept: a verb goes by its default name
/datum/a/proc/shared_name(x)
	return "shared[x]"
/datum/a/verb/sharedname(x as num)
	set name = "shared name"
	return "sharedverb[x]"

// kept: players see the names of verbs, and of procs that become verbs
/obj/holder/verb/held_verb()
	return "held"
/obj/holder/proc/added_later(x)
	return "added[x]"
/obj/holder/proc/not_added(x)
	NAME_IT("not_added")
	return "not added[x]"

/world/proc/world_proc(x)
	NAME_IT("world_proc")
	return "world[x]"
// a global proc is in no type's table: left as it is
/proc/global_one(x)
	return "global[x]"
/proc/global_fails()
	var/list/nothing = null
	return nothing[1]

/proc/same(thing)
	return thing

/proc/attempt(datum/thing, how, name)
	try
		switch(how)
			if("call")
				return call(same(thing), name)(7)
			if("hascall")
				return hascall(thing, name)
	catch(var/exception/e)
		return "runtime error: [e.name]"

/proc/error_text(datum/a/thing, global_proc)
	try
		if(global_proc)
			global_fails()
		else
			thing.will_fail()
	catch(var/exception/e)
		return splittext("[e.desc]", "\n")[1]

/proc/results()
	var/datum/a/b/c/thing = new
	thing.other = thing
	var/datum/other/other = new
	var/untyped = thing
	var/untyped_other = other
	OUT("plain: typed [thing.plain_proc(1)] untyped [untyped:plain_proc(2)] implicit [thing.calls_itself(3)]")
	OUT("plain on another type: typed [other.plain_proc(1)] untyped [untyped_other:plain_proc(2)]")
	OUT("plain by name: [attempt(thing, "call", "plain_proc")] hascall [attempt(thing, "hascall", "plain_proc")]")
	OUT("settings: [thing.does_not_wait(1)] [thing.in_background(2)] [untyped:does_not_wait(3)]")
	OUT("text: [thing.has_text(1)] [untyped:has_text(2)] [attempt(thing, "call", "has text")] [attempt(thing, "call", "has_text")] hascall [attempt(thing, "hascall", "has text")]")
	OUT("verbs: [thing.a_verb(1)] [thing.with_category(2)] [thing.with_desc(3)] [thing.is_hidden(4)] [untyped:with_category(5)]")
	OUT("named: [thing.custom_named(1)] [untyped:custom_named(2)] [attempt(thing, "call", "Custom")]")
	var/as_value = /datum/a/proc/used_as_value
	OUT("value: [thing.used_as_value(1)] [call(thing, as_value)(2)] [untyped:used_as_value(3)]")
	OUT("shared: [thing.shared_name(1)] [thing.sharedname(2)] [untyped:shared_name(3)]")
	OUT("world: [world.world_proc(1)] global: [global_one(2)]")
	var/obj/holder/holder = new
	holder.verbs += /obj/holder/proc/added_later
	var/list/verb_names = list()
	for(var/held in holder.verbs)
		verb_names += "[held:name]"
	OUT("verb names: [verb_names.Join(", ")]; [holder.added_later(1)] [holder.not_added(2)]")
	OUT("error: [error_text(thing, 0)]")
	OUT("global error: [error_text(thing, 1)]")

/world/New()
	..()
	fdel("result.txt")
	try
		results()
	catch(var/exception/e)
		OUT("runtime error: [e.name]")
	OUT("done")
	shutdown()
