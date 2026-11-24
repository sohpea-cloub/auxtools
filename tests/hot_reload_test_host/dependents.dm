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
