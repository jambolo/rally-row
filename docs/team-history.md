# Franchise identity and historical consistency

See [franchise continuity](model.md#franchise-continuity) for rating behavior,
[configuration](extending.md#configuration) for era fields and structural validation, and
[`config/nfl.json`](../config/nfl.json) and [`config/mlb.json`](../config/mlb.json) for each league's
identity ranges and aliases.

## Identity details

- Permanent IDs initially resemble current abbreviations. A future relocation keeps the franchise's ID even
  if the provider adopts a new abbreviation: the Raiders would still use `LV` and the Athletics `ATH`.
- Identity ranges begin at each league's historical cutoff (NFL 2002, MLB 1998), not at franchise founding.
- Renames within a season would require extending the registry to effective dates.
- Source ids must be valid in the game's era for both leagues.
- NFL source ids are the provider's era-specific abbreviations. An `LV` provider code in a 2019 game is
  rejected even though `LV` is the permanent franchise ID.
- MLB source ids are Stats API numeric team ids, which stay the same across renames and relocations, so
  every era of a franchise lists the same id; for example, `120` for both the Montreal Expos and the
  Washington Nationals.
- Team estimates resolve names and abbreviations from the model's target season. An era's abbreviation
  defaults to its first source id, so NFL eras omit it; MLB eras must set it, such as `MON` for the
  Montreal Expos and `WSH` for the Washington Nationals.

## Identity events

Configured eras for franchises whose name or home market changed during the modeled history. An open-ended
era runs through the current season.

### NFL

| Franchise ID | Source ids | Eras |
| --- | --- | --- |
| `LAC` | `SD`, then `LAC` | San Diego Chargers, San Diego (2002–2016); Los Angeles Chargers, Los Angeles (2017–) |
| `LAR` | `STL`, then `LA` and `LAR` | St. Louis Rams, St. Louis (2002–2015); Los Angeles Rams, Los Angeles (2016–) |
| `LV` | `OAK`, then `LV` | Oakland Raiders, Oakland (2002–2019); Las Vegas Raiders, Las Vegas (2020–) |
| `WAS` | `WAS` and `WSH` | Washington Redskins (2002–2019); Washington Football Team (2020–2021); Washington Commanders (2022–) |

### MLB

| Franchise ID | Source id | Abbreviations | Eras |
| --- | --- | --- | --- |
| `WSH` | `120` | `MON`, then `WSH` | Montreal Expos, Montreal (1998–2004); Washington Nationals, Washington (2005–) |
| `LAA` | `108` | `ANA`, then `LAA` | Anaheim Angels, Anaheim (1998–2004); Los Angeles Angels of Anaheim, Los Angeles (2005–2015); Los Angeles Angels, Los Angeles (2016–) |
| `TB` | `139` | `TB` | Tampa Bay Devil Rays, Tampa Bay (1998–2007); Tampa Bay Rays, Tampa Bay (2008–) |
| `MIA` | `146` | `FLA`, then `MIA` | Florida Marlins, Florida (1998–2011); Miami Marlins, Miami (2012–) |
| `CLE` | `114` | `CLE` | Cleveland Indians, Cleveland (1998–2021); Cleveland Guardians, Cleveland (2022–) |
| `ATH` | `133` | `OAK`, then `ATH` | Oakland Athletics, Oakland (1998–2024); Athletics, Sacramento (2025–) |

## Registry consistency

The browser's current-season cache embeds the `teams` registry described in the
[historical output format](extending.md#historical-output). The Elo seed contains ratings and
configuration/history hashes, not a copy of the registry.

The Elo calculator rejects historical files whose embedded identity registry differs from the active
configuration. Regeneration commands are in [Development](../DEVELOPMENT.md#data-and-configuration).

## Historical sources

### NFL

Sources for the configured NFL identity transitions:

- [Raiders history](https://www.profootballhof.com/teams/las-vegas-raiders/team-history)
- [Rams history](https://www.profootballhof.com/teams/los-angeles-rams/team-history)
- [Chargers' first return season in Los Angeles](https://en.wikipedia.org/wiki/2017_Los_Angeles_Chargers_season)
- [Washington history](https://www.profootballhof.com/teams/washington-commanders/team-history)

### MLB

The Stats API teams endpoint reports a franchise's name and location for a given season. Sources for the
configured MLB identity transitions:

- Expos to Nationals: the API reports the Montreal Expos for
  [2004](https://statsapi.mlb.com/api/v1/teams/120?season=2004) and the Washington Nationals for
  [2005](https://statsapi.mlb.com/api/v1/teams/120?season=2005); see also the
  [Montreal Expos history](https://en.wikipedia.org/wiki/Montreal_Expos).
- Angels: the API reports the Anaheim Angels for [2004](https://statsapi.mlb.com/api/v1/teams/108?season=2004)
  and "Los Angeles Angels" for 2005 and later; it does not list the 2005–2015 name. The
  [Los Angeles Angels history](https://en.wikipedia.org/wiki/Los_Angeles_Angels) documents the 2005 rename
  to Los Angeles Angels of Anaheim and the 2016 rename to Los Angeles Angels.
- Devil Rays to Rays: the API reports the Tampa Bay Devil Rays for
  [2007](https://statsapi.mlb.com/api/v1/teams/139?season=2007) and the Tampa Bay Rays for
  [2008](https://statsapi.mlb.com/api/v1/teams/139?season=2008); see also the
  [Tampa Bay Rays history](https://en.wikipedia.org/wiki/Tampa_Bay_Rays).
- Florida to Miami Marlins: the API reports the Florida Marlins for
  [2011](https://statsapi.mlb.com/api/v1/teams/146?season=2011) and the Miami Marlins for
  [2012](https://statsapi.mlb.com/api/v1/teams/146?season=2012); see also the
  [Miami Marlins history](https://en.wikipedia.org/wiki/Miami_Marlins).
- Indians to Guardians: the API reports the Cleveland Indians for
  [2021](https://statsapi.mlb.com/api/v1/teams/114?season=2021) and the Cleveland Guardians for
  [2022](https://statsapi.mlb.com/api/v1/teams/114?season=2022); see also the
  [Cleveland Guardians history](https://en.wikipedia.org/wiki/Cleveland_Guardians).
- Oakland Athletics to Athletics in Sacramento: the API reports the Oakland Athletics for
  [2024](https://statsapi.mlb.com/api/v1/teams/133?season=2024) and the Athletics, located in Sacramento,
  for [2025](https://statsapi.mlb.com/api/v1/teams/133?season=2025); see also the
  [Athletics history](https://en.wikipedia.org/wiki/Athletics_%28baseball%29).
- History start: the [1998 Major League Baseball expansion](https://en.wikipedia.org/wiki/1998_Major_League_Baseball_expansion)
  added the Arizona Diamondbacks and Tampa Bay Devil Rays, so all 30 franchises exist throughout the MLB history.
