//! Deliberately small launch catalog. Add airports with their IANA timezone here.
#[derive(Clone, Copy)]
pub(crate) struct Airport {
    pub code: &'static str,
    pub name: &'static str,
    pub timezone: &'static str,
}

#[derive(Clone, Copy)]
pub(crate) struct Airline {
    pub code: &'static str,
    pub name: &'static str,
}

pub(crate) const AIRPORTS: &[Airport] = &[
    Airport { code: "RDU", name: "Raleigh-Durham International", timezone: "America/New_York" },
    Airport { code: "ATL", name: "Hartsfield-Jackson Atlanta International", timezone: "America/New_York" },
    Airport { code: "BOS", name: "Boston Logan International", timezone: "America/New_York" },
    Airport { code: "BWI", name: "Baltimore/Washington International Thurgood Marshall", timezone: "America/New_York" },
    Airport { code: "CLT", name: "Charlotte Douglas International", timezone: "America/New_York" },
    Airport { code: "DCA", name: "Ronald Reagan Washington National", timezone: "America/New_York" },
    Airport { code: "DEN", name: "Denver International", timezone: "America/Denver" },
    Airport { code: "DFW", name: "Dallas Fort Worth International", timezone: "America/Chicago" },
    Airport { code: "DTW", name: "Detroit Metropolitan Wayne County", timezone: "America/Detroit" },
    Airport { code: "EWR", name: "Newark Liberty International", timezone: "America/New_York" },
    Airport { code: "FLL", name: "Fort Lauderdale-Hollywood International", timezone: "America/New_York" },
    Airport { code: "IAD", name: "Washington Dulles International", timezone: "America/New_York" },
    Airport { code: "IAH", name: "George Bush Intercontinental", timezone: "America/Chicago" },
    Airport { code: "JFK", name: "John F. Kennedy International", timezone: "America/New_York" },
    Airport { code: "LAS", name: "Harry Reid International", timezone: "America/Los_Angeles" },
    Airport { code: "LAX", name: "Los Angeles International", timezone: "America/Los_Angeles" },
    Airport { code: "LGA", name: "LaGuardia", timezone: "America/New_York" },
    Airport { code: "MCO", name: "Orlando International", timezone: "America/New_York" },
    Airport { code: "MIA", name: "Miami International", timezone: "America/New_York" },
    Airport { code: "MSP", name: "Minneapolis-Saint Paul International", timezone: "America/Chicago" },
    Airport { code: "ORD", name: "Chicago O'Hare International", timezone: "America/Chicago" },
    Airport { code: "PHL", name: "Philadelphia International", timezone: "America/New_York" },
    Airport { code: "PHX", name: "Phoenix Sky Harbor International", timezone: "America/Phoenix" },
    Airport { code: "SAN", name: "San Diego International", timezone: "America/Los_Angeles" },
    Airport { code: "SEA", name: "Seattle-Tacoma International", timezone: "America/Los_Angeles" },
    Airport { code: "SFO", name: "San Francisco International", timezone: "America/Los_Angeles" },
    Airport { code: "SLC", name: "Salt Lake City International", timezone: "America/Denver" },
    Airport { code: "TPA", name: "Tampa International", timezone: "America/New_York" },
];

pub(crate) const AIRLINES: &[Airline] = &[
    Airline { code: "AA", name: "American Airlines" },
    Airline { code: "AS", name: "Alaska Airlines" },
    Airline { code: "B6", name: "JetBlue" },
    Airline { code: "DL", name: "Delta Air Lines" },
    Airline { code: "F9", name: "Frontier Airlines" },
    Airline { code: "NK", name: "Spirit Airlines" },
    Airline { code: "UA", name: "United Airlines" },
    Airline { code: "WN", name: "Southwest Airlines" },
];

pub(crate) fn airport(code: &str) -> Option<&'static Airport> {
    AIRPORTS.iter().find(|airport| airport.code.eq_ignore_ascii_case(code.trim()))
}

pub(crate) fn airline(code: &str) -> Option<&'static Airline> {
    AIRLINES.iter().find(|airline| airline.code.eq_ignore_ascii_case(code.trim()))
}
