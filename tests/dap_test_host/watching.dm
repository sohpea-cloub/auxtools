var/watched_number = 1
var/list/watched_list = list("a", "b", "c")
var/list/watched_pairs = list("one" = 1, "two" = 2)

/proc/change_number(by)
	watched_number += by
	return watched_number // @change_number_after

/datum/thing/proc/rename(called)
	name = called
	return src // @rename_after

/proc/sc_watch()
	var/datum/thing/kept = new("kept")
	var/steps // @watch_start
	steps = 1
	change_number(2) // @watch_call
	watched_list[2] = "B"
	watched_pairs["two"] = 22 // @watch_pair
	kept.rename("renamed") // @watch_object
	kept.n = kept.n
	steps += 10
	sleep(1) // @watch_sleep
	change_number(3)
	steps += 100
	del(kept) // @watch_del
	return steps

/proc/churn(depth)
	if (depth > 0)
		return churn(depth - 1) + 1
	return 0

/proc/sc_watch_long()
	var/patient = 1 // @watch_long_start
	for (var/i in 1 to 30)
		spawn(0)
			churn(5)
		churn(3)
		sleep(1)
	patient = 2
	return patient
