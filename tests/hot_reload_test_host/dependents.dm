/datum/dep_made

/datum/dep_made/other

/datum/dep_holder
	var/datum/dep_made/item = new

var/dep_item_type = /datum/dep_made
var/dep_count = 100

/datum/dep_factory

/datum/dep_factory/proc/make()
	return new dep_item_type()

/datum/dep_factory/proc/first()
	for(dep_count in list(7))
		break

/proc/dep_factory_counts()
	var/datum/factory = new /datum/dep_factory
	factory:first()
	return "[factory.vars["dep_count"]] [dep_count]"

var/const/DEP_LIMIT = 5

/datum/dep_limited
	var/list/limits = list(DEP_LIMIT + 1)

/mob/dep_chooser/verb/choose_dep_limit(x as anything in list(DEP_LIMIT + 1))
	return x

/datum/dep_called/proc/f()
	return "declaration"

/datum/dep_called/f()
	return "override"

/datum/dep_config
	var/handler = /datum/dep_called::f()

/datum/dep_copier

var/const/DEP_STEP = 5

/datum/dep_twice/proc/f()
	return 0

/datum/dep_twice/f()
	return 1

/datum/dep_twice/f()
	return 2

/proc/dep_next()
	var/last = /datum/dep_twice::f()
	return last ? DEP_STEP + 1 : 0

/datum/dep_searched/proc/speak()
	return "searched declaration"

/datum/dep_searched/speak()
	return "searched first override"

/datum/dep_searched/speak()
	return "searched second override"

/datum/dep_searched/child

/proc/dep_calls_searched()
	var/last = /datum/dep_twice::f()
	return last ? call(new /datum/dep_searched, /datum/dep_searched/child.proc/speak)() : null

var/dep_total = 100

/datum/dep_counted

/datum/dep_counted/proc/add(dep_total)
	var/last = /datum/dep_twice::f()
	return last ? dep_total + 1 : 0

var/const/DEP_QUALIFIED = 5

/proc/dep_qualified()
	var/last = /datum/dep_twice::f()
	return last ? ::DEP_QUALIFIED + 1 : 0

/proc/dep_dotted()
	var/last = /datum/dep_twice::f()
	return last ? global.DEP_QUALIFIED + 1 : 0

/datum/dep_static_config
	var/static/handler = /datum/dep_called::f()

/datum/dep_static_copier

/proc/dep_change_static_handler()
	var/datum/dep_static_config/config = new
	config.handler = "changed"
	return config.handler

/datum/dep_choices_a
	var/static/list/choices = list("a")

/datum/dep_choices_b
	var/static/list/choices = list("b")

/mob/dep_targeted
	var/datum/dep_choices_a/target

/mob/dep_targeted/verb/choose_dep_target(x as anything in target::choices)
	return x
