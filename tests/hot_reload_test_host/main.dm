/early_global()
	return "early unreachable"

/proc/auxtools_launched_world_del()
	CRASH("auxtools not loaded")

/world/New()
	spawn()
		sleeper()
	spawn()
		main_loop()
	. = ..()

var/reboot_requested = FALSE

/proc/request_reboot()
	reboot_requested = TRUE

/world
	fps = 100

/proc/main_loop()
	while (!reboot_requested)
		tick()
		sleep(world.tick_lag)

	world.Reboot()

/datum/limits
	var/const/MAX = 3

/datum/limits/proc/max_doubled()
	return MAX * 2

var/const/LIMIT_ALL = 10

/proc/limit_all()
	return list(LIMIT_ALL, LIMIT_ALL + 1)

var/datum/limits/kept_limits = new

/datum/limits/var/list/things = null

/datum/limits/proc/thing_count()
	return length(things)

/datum/limits/proc/label()
	set name = "Label"
	return "labelled"

/datum/chain/proc/say()
	return "1"

/datum/chain/say()
	return ..() + "3"

var/datum/chain/the_chain = new

/mob/chain_verbs/verb/speak()
	return "a"

/mob/chain_verbs/verb/other()
	return "o"

/mob/chain_verbs/speak()
	return ..() + "c"

var/mob/chain_verbs/verb_chain = new

/proc/verb_names(mob/m)
	var/list/names = list()
	for (var/v in m.verbs)
		names += "[v:name]"
	return jointext(names, ",")

/world/proc/chant()
	return "x"

/world/chant()
	return ..() + "z"

/world/proc/hail()
	set name = "Hail"
	return "hailed"

/proc/world_call(name)
	try
		return call(world, name)()
	catch
		return "missing"

/proc/solo()
	return "declared"

/solo()
	return "another"

/datum/declared
	var/count = 1
	var/shared = 10
	var/many = null

/datum/declared/sub

/datum/declared/proc/read_count()
	return count

/datum/declared/proc/read_shared()
	return shared

/datum/declared/proc/many_count()
	return length(many)

var/datum/declared/declared_one = new

/proc/compiled_late()
	return 0

/proc/keyed(a, b)
	return b

/datum/hr_named
	var/static/falloff = 9

/proc/hr_named_falloff(sound/s)
	var/datum/hr_named/named = new
	. = "[s.falloff] "
	s.falloff = 7
	. += "[s.falloff] [named.falloff]"

/sound/hr_sa
	pan = 10

/sound/hr_sb
	priority = 5

/sound/hr_st
	parent_type = /sound/hr_sa
	frequency = 3

/datum/early_apart/speak()
	return "override apart"

/proc/calls_twice_unchanged()
	return call(new /datum/twice, /datum/twice/speak)()

/datum/names_twice
	var/said = /datum/twice/speak
	var/list/all_said = list(/datum/twice/speak)

/proc/calls_inserted_unchanged()
	return call(new /datum/inserted, /datum/inserted/speak)()

/proc/reads_twice_default()
	return /datum/names_twice::said

/datum/reads_names_twice
	var/list/all = list(/datum/names_twice::said)

/mob/chooser/verb/choose_twice(x in list(/datum/twice/speak))
	return x

/proc/calls_kin_child()
	return call(new /datum/kin/middle/child, /datum/kin/middle/child::speak())()

/proc/calls_lone_unchanged()
	return call(new /datum/lone, /datum/lone/speak)()

/datum/names_lone
	var/said = /datum/lone/speak

/datum/gone

/proc/calls_gone_unchanged()
	return call(new /datum/gone, /datum/gone/proc/speak)()

/datum/names_gone
	var/gone_said = /datum/gone/proc/speak

/proc/calls_last_unchanged()
	return call(new /datum/last, /datum/last::speak())()

/datum/names_last
	var/said = /datum/last::speak()
