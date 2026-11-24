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

/datum/dep_counted/proc/add_default(dep_total, limit = dep_total)
	var/last = /datum/dep_twice::f()
	return last ? limit + 1 : 0

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

var/const/DEP_OWN = 5

/datum/dep_own
	var/const/DEP_OWN = 2

/datum/dep_own/proc/own()
	return DEP_OWN + 1

/proc/dep_own_global()
	return DEP_OWN + 1

/datum/dep_own/proc/own_alone()
	return DEP_OWN

/proc/dep_own_global_alone()
	return DEP_OWN

/datum/dep_own/proc/next()
	var/last = /datum/dep_twice::f()
	return last ? DEP_OWN + 1 : 0

/datum/dep_member_config
	var/const/DEP_MEMBER = 5

/proc/dep_member_chain(datum/dep_member_config/config)
	var/last = /datum/dep_twice::f()
	return last ? config.DEP_MEMBER + 1 : 0

var/const/DEP_ALONE = 5

/proc/dep_alone_sum()
	var/last = /datum/dep_twice::f()
	return last ? ::DEP_ALONE + 1 : 0

/proc/dep_alone()
	var/last = /datum/dep_twice::f()
	return last ? ::DEP_ALONE : 0

/proc/dep_local_limit()
	var/const/DEP_LOCAL = 5
	return "[DEP_LOCAL] [DEP_LOCAL + 1]"

var/const/DEP_LOCAL_BASE = 5

/proc/dep_local_based()
	var/const/DEP_LOCAL_SUM = DEP_LOCAL_BASE + 1
	return DEP_LOCAL_SUM

var/const/DEP_OUTPUT = 5

/proc/dep_output()
	var/last = /datum/dep_twice::f()
	var/savefile/S = new
	S["v"] << DEP_OUTPUT
	var/v
	S["v"] >> v
	return last ? v : 0

/datum/dep_global_config
	var/const/DEP_GLOBAL_MEMBER = 5

var/datum/dep_global_config/dep_global_config

/proc/dep_global_chain()
	var/last = /datum/dep_twice::f()
	return last ? global.dep_global_config.DEP_GLOBAL_MEMBER + 1 : 0

var/const/DEP_OFFSET_BASE = 5

/proc/dep_local_offset()
	var/const/DEP_OFFSET = 1
	var/const/DEP_OFFSET_SUM = DEP_OFFSET_BASE + DEP_OFFSET
	return DEP_OFFSET_SUM

/datum/dep_local_config
	var/dep_limit = 5

/proc/dep_local_default()
	var/const/DEP_DEFAULT_LIMIT = /datum/dep_local_config::dep_limit
	return DEP_DEFAULT_LIMIT

/datum/dep_colon_later

var/const/DEP_SCOPED_BASE = 5

/datum/dep_scoped_config
	var/const/DEP_SCOPED = 5

var/const/DEP_PARENT = 5

/datum/dep_parent/proc/f(x)
	return x

/datum/dep_colon_list_first
	var/const/DEP_COLON_LIST = 5

/datum/dep_colon_list_later

/datum/dep_colon_alone_first
	var/const/DEP_COLON_ALONE = 5

/datum/dep_colon_alone_later

/datum/dep_retype_first
	var/const/DEP_RETYPE = 5

/datum/dep_retype_later
	var/const/DEP_RETYPE = 9

var/datum/dep_retype_first/dep_retype_target

/datum/dep_colon_indexed_first
	var/const/DEP_COLON_INDEXED = 5

/datum/dep_colon_indexed_later

/datum/dep_retype_alone_first
	var/const/DEP_RETYPE_ALONE = 5

/datum/dep_retype_alone_later
	var/const/DEP_RETYPE_ALONE = 9

var/datum/dep_retype_alone_first/dep_retype_alone_target

/datum/dep_safe_index_a
	var/const/DEP_SAFE_INDEX = 5

/datum/dep_safe_index_b
	var/const/DEP_SAFE_INDEX = 9

/datum/dep_safe_index_holder
	var/list/datum/dep_safe_index_a/items = list(new /datum/dep_safe_index_a)

/proc/dep_safe_index(datum/dep_safe_index_holder/holder)
	return holder?.items[1].DEP_SAFE_INDEX

/datum/dep_call_colon_first
	var/const/DEP_CALL_COLON = 5

/datum/dep_call_colon_later

/datum/dep_call_colon_factory/proc/make()
	return new /datum/dep_call_colon_first

/datum/dep_returned_first
	var/const/DEP_RETURNED = 5

/datum/dep_returned_later
	var/const/DEP_RETURNED = 9

/proc/dep_returned_make()
	return new /datum/dep_returned_first

/datum/dep_bare_call_first
	var/const/DEP_BARE_CALL = 5

/datum/dep_bare_call_later

/proc/dep_bare_make()
	return new /datum/dep_bare_call_first

/datum/dep_safe_write_a
	var/static/DEP_SAFE_WRITE = 1

/datum/dep_safe_write_b
	var/static/DEP_SAFE_WRITE = 2

/datum/dep_safe_write_factory/proc/make()
	return new /datum/dep_safe_write_a

/proc/dep_safe_write(value)
	var/datum/dep_safe_write_factory/factory = new
	factory.make()?.DEP_SAFE_WRITE = value
	var/datum/dep_safe_write_a/a = new
	var/datum/dep_safe_write_b/b = new
	return "[a.DEP_SAFE_WRITE] [b.DEP_SAFE_WRITE]"

/datum/dep_safe_colon_a
	var/const/DEP_SAFE_COLON = 5

/datum/dep_safe_colon_b
	var/const/DEP_SAFE_COLON = 9

/datum/dep_safe_colon_base

/datum/dep_safe_colon_base/sub/proc/make() as /datum/dep_safe_colon_a
	return new /datum/dep_safe_colon_b

/proc/dep_safe_colon(datum/dep_safe_colon_base/base)
	return base?:make().DEP_SAFE_COLON
