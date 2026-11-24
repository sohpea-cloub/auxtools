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

/proc/calls_kept_declaration()
	var/last = /datum/kept::speak()
	return "[call(new /datum/kept, /datum/kept/proc/speak)()], then [call(new /datum/kept, last)()]"

var/hr_tally = 100

/datum/tallied/proc/bump()
	hr_tally++

/proc/tallies()
	var/datum/tallied/T = new
	T.bump()
	return "[("hr_tally" in T.vars) ? T.vars["hr_tally"] : "none"] [global.hr_tally]"

/proc/hr_asked()
	return "global"

/datum/asking/proc/ask()
	return hr_asked()

/datum/asking/child

/datum/config_read
	var/list/limits = list(/datum/config_limits::limit)
	var/copied = /datum/config_limits::limit

/mob/limited/verb/choose_limit(x in list(/datum/config_limits::limit))
	return x

/datum/dep_colon_first
	var/const/DEP_COLON = 5

/proc/dep_colon(x)
	return "[x:DEP_COLON + 1] [x:DEP_COLON]"

/proc/dep_scoped_limit(x)
	if(x)
		var/const/DEP_SCOPED_BASE = 2
		x = DEP_SCOPED_BASE
	var/const/DEP_SCOPED_LIMIT = DEP_SCOPED_BASE + 1
	return DEP_SCOPED_LIMIT

/proc/dep_typed_limit(datum/dep_scoped_config/config)
	var/const/DEP_TYPED_LIMIT = config.DEP_SCOPED + 1
	return DEP_TYPED_LIMIT

/datum/dep_parent/child/f(x)
	var/last = /datum/dep_twice::f()
	return last ? ..(DEP_PARENT) : 0

/proc/dep_self(x)
	var/last = /datum/dep_twice::f()
	return x || !last ? x : .(DEP_PARENT)

/mob/dep_colon_chooser
	var/dep_colon_holder

/mob/dep_colon_chooser/verb/choose_dep_colon_list(x as anything in list(dep_colon_holder:DEP_COLON_LIST + 1))
	return x

/datum/dep_colon_inits
	var/dep_colon_holder
	var/list/limits = list(dep_colon_holder:DEP_COLON_LIST + 1)

/proc/dep_colon_alone(x)
	var/last = /datum/dep_twice::f()
	return last ? x:DEP_COLON_ALONE : 0

/proc/dep_colon_local_const(x)
	var/const/DEP_COLON_LOCAL = x:DEP_COLON_LIST + 1
	return DEP_COLON_LOCAL

/proc/dep_retyped_local_const()
	var/const/DEP_RETYPED_LOCAL = dep_retype_target.DEP_RETYPE + 1
	return DEP_RETYPED_LOCAL
