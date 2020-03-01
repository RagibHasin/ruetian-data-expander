#![deny(nonstandard_style, unused, future_incompatible)]
#![feature(range_is_empty)]
#![feature(map_first_last)]

use chrono::prelude::*;
use serde::{Deserialize, Serialize};
use serde_yaml as yaml;
use std::{
    cmp::*,
    collections::{hash_map::*, *},
    fs::*,
    str::FromStr,
    time::SystemTime,
};

use ruetian_common::*;

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
struct CacheDept {
    pub semester: SystemTime,
    pub notices: Option<SystemTime>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
struct Cache {
    pub inner: HashMap<u8, HashMap<String, CacheDept>>,
    pub days: SystemTime,
    pub holidays: SystemTime,
    pub central_notices: SystemTime,
}

fn main() -> Result<()> {
    let (days_map_time, days_map) = match File::open("data/days.yaml") {
        Ok(file) => (
            file.metadata()?.modified()?,
            yaml::from_reader::<_, BTreeMap<NaiveDate, Day>>(file)?,
        ),
        Err(e) => return Err(Box::new(e)),
    };

    let (holidays_time, holidays) = match File::open("data/holidays.yaml") {
        Ok(file) => (
            file.metadata()?.modified()?,
            yaml::from_reader::<_, Vec<Holiday>>(file)?,
        ),
        Err(e) => return Err(Box::new(e)),
    };

    let (central_notices_time, central_notices) = match File::open("data/notices.yaml") {
        Ok(file) => (
            file.metadata()?.modified()?,
            yaml::from_reader::<_, Vec<Notice>>(file)?,
        ),
        Err(e) => return Err(Box::new(e)),
    };

    let mut cache: Cache = File::open("cache.yaml").map_or(
        Cache {
            inner: HashMap::new(),
            days: days_map_time,
            holidays: holidays_time,
            central_notices: central_notices_time,
        },
        |file| yaml::from_reader(file).unwrap(),
    );

    for series_dir in read_dir("data")? {
        let series_dir = series_dir?;
        if !series_dir.metadata()?.is_dir() {
            continue;
        }
        let series_dir_name = series_dir.file_name();
        let series = u8::from_str(series_dir_name.to_str().unwrap())?;
        let series_cache = cache.inner.entry(series).or_insert_with(HashMap::new);

        for dir in series_dir
            .path()
            .read_dir()?
            .filter_map(|dir| dir.ok())
            .chain(std::iter::once(series_dir))
        {
            let mut semester_path = dir.path().clone();
            if !semester_path.is_dir() {
                continue;
            }
            semester_path.push("semester.yaml");
            if semester_path.exists() {
                let (local_notices_time, local_notices) = match File::open({
                    let mut path = dir.path().clone();
                    path.push("notices.yaml");
                    path
                }) {
                    Ok(file) => (
                        Some(file.metadata()?.modified()?),
                        Some(yaml::from_reader::<_, Vec<Notice>>(file)?),
                    ),
                    Err(_) => (None, None),
                };
                let dept = dir.file_name().to_str().unwrap().to_owned();
                let semester_time = metadata(&semester_path)?.modified()?;
                match series_cache.entry(dept) {
                    Entry::Occupied(entry) => {
                        if entry.get().semester == semester_time
                            && entry.get().notices == local_notices_time
                            && cache.days == days_map_time
                            && cache.holidays == holidays_time
                            && cache.central_notices == central_notices_time
                        {
                            continue;
                        }
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(CacheDept {
                            semester: semester_time,
                            notices: local_notices_time,
                        });
                    }
                }
                let start_date: NaiveDate = yaml::from_slice(&read(semester_path)?)?;
                /*
                let stringed = yaml::to_string(&get_dates_mapped(
                    &days_map,
                    &holidays,
                    &central_notices,
                    start_date,
                )?)?;
                write(
                    {
                        let mut path = dir.path().clone();
                        path.push("dates_map.yaml");
                        path
                    },
                    &stringed,
                )?;*/
                yaml::to_writer(
                    File::create({
                        let mut path = dir.path().clone();
                        path.push("dates_map.yaml");
                        path
                    })?,
                    &get_dates_mapped(
                        &days_map,
                        &holidays,
                        &central_notices,
                        &local_notices,
                        start_date,
                    )?,
                )?;
            }
        }
    }

    yaml::to_writer(File::create("cache.yaml")?, &cache)?;

    Ok(())
}

fn get_dates_mapped(
    days_map: &BTreeMap<NaiveDate, Day>,
    holidays: &[Holiday],
    notices: &[Notice],
    local_notices: &Option<Vec<Notice>>,
    start_date: NaiveDate,
) -> Result<BTreeMap<NaiveDate, (Day, u32)>> {
    let days_map = days_map
        .range((days_map.range(..start_date).nth_back(0).unwrap().0)..)
        .map(|(date, day)| (*date, *day))
        .collect::<BTreeMap<_, _>>();

    let (date, day) = days_map.first_key_value().unwrap();
    let (mut date, mut day) = (date.succ(), *day);
    let first_assigned_date = date.clone();
    let _s_first_assigned_date = first_assigned_date.to_string();

    let mut holidays = holidays
        .iter()
        .map(|holiday| holiday.start..=holiday.end)
        .filter(|range| &first_assigned_date < range.start());

    let mut day_offs = notices.iter().filter_map(|notice| match notice {
        Notice::ClassOff {
            day_off: true,
            date,
            time: TimeScope::AllDay(end_date),
            ..
        } if &first_assigned_date < date => Some((date, end_date)),
        _ => None,
    });

    let mut local_day_offs = local_notices
        .iter()
        .flatten()
        .filter_map(|notice| match notice {
            Notice::ClassOff {
                day_off: true,
                date,
                time: TimeScope::AllDay(end_date),
                ..
            } if &first_assigned_date < date => Some((date, *end_date)),
            Notice::Others { date, message }
                if &first_assigned_date < date && message == "Mid-semester break" =>
            {
                Some((date, Some(*date + chrono::Duration::days(4))))
            }
            _ => None,
        });

    let mut upcoming_holidays = holidays.next();
    let mut upcoming_day_offs = day_offs.next();
    let mut upcoming_local_day_offs = local_day_offs.next();
    let mut cycle = 1;
    let mut days = 0;
    let mut dates_map = BTreeMap::<NaiveDate, (Day, u32)>::new();
    while !(days >= 65 && date.weekday() == Weekday::Thu) {
        let _date_as_str = date.to_string();
        match days_map.get(&date) {
            None => {
                if let Some(ref current_holidays) = upcoming_holidays {
                    if current_holidays.contains(&date) {
                        date = current_holidays.end().succ();
                        upcoming_holidays = holidays.next();
                        continue;
                    }
                }
                if let Some(ref current_day_offs) = upcoming_day_offs {
                    if let Some(end_date) = current_day_offs.1 {
                        if &date >= current_day_offs.0 && &date <= end_date {
                            date = end_date.succ();
                            upcoming_day_offs = day_offs.next();
                            continue;
                        }
                    } else {
                        break;
                    }
                }
                if let Some(ref current_local_day_offs) = upcoming_local_day_offs {
                    if let Some(end_date) = current_local_day_offs.1 {
                        if &date >= current_local_day_offs.0 && date <= end_date {
                            date = end_date.succ();
                            upcoming_local_day_offs = local_day_offs.next();
                            continue;
                        }
                    } else {
                        break;
                    }
                }
                match date.weekday() {
                    Weekday::Thu => {
                        date = date.succ().succ();
                        continue;
                    }
                    Weekday::Fri => {
                        date = date.succ();
                        continue;
                    }
                    _ => {
                        if date >= start_date {
                            // Everything OK. This day is real.
                            dates_map.insert(date, (day.succ_mut(), cycle));
                            days += 1;
                        }
                    }
                }
            }
            Some(assigned_day) => {
                day = *assigned_day;
                dates_map.insert(date, (day, cycle));
                days += 1;
            }
        }

        if day == Day::E {
            cycle += 1;
        }
        date = date.succ();
    }

    Ok(dates_map)
}
