import { formatOdds, formatRecord, isListed, statusText, type PostseasonState } from './postseason.ts';

/** Renders the odds the refresh worker computed; the page never simulates. */
export function PostseasonOdds({
  odds,
  label,
  teamName,
}: {
  odds: PostseasonState;
  label: string;
  teamName: (id: string) => string;
}) {
  return (
    <section className="panel postseason-odds" aria-labelledby="postseason-heading">
      <div className="section-heading">
        <div>
          <p className="eyebrow">Postseason odds</p>
          <h2 id="postseason-heading">Who goes all the way?</h2>
        </div>
        {odds.status === 'ready' && <span className="chip">{odds.mode === 'regular' ? 'Regular season' : label}</span>}
      </div>
      {odds.status === 'error' ? (
        <div className="notice error">
          <strong>Postseason odds are unavailable.</strong>
          <details>
            <summary>Technical details</summary>
            <p>{odds.error}</p>
          </details>
        </div>
      ) : (
        <>
          <p className="table-note">
            Based on {odds.simulations.toLocaleString()} simulations of the remaining games. Seed is the average seed when the team
            makes the playoffs. The dashed line marks the last playoff spot. Tiebreakers use only wins, losses, and ties; ties they
            can't break are decided by random draws.{' '}
            {odds.mode === 'regular'
              ? 'Teams with less than a 0.1% chance of making the playoffs are not listed.'
              : 'Teams that missed the playoffs or have been eliminated are not listed.'}
          </p>
          {odds.notes.length > 0 && (
            <ul className="odds-notes">
              {odds.notes.map((note) => (
                <li key={note}>{note}</li>
              ))}
            </ul>
          )}
          {odds.conferences.map((conference) => {
            const teams = conference.teams.filter((team) => isListed(team, odds.mode));
            if (teams.length === 0) return null;
            return (
              <div key={conference.id}>
                <h3>{conference.name}</h3>
                <div className="table-wrap" tabIndex={0} role="region" aria-label={`${conference.name} postseason odds`}>
                  <table className="odds-table">
                    <thead>
                      <tr>
                        <th scope="col">Team</th>
                        <th scope="col">Record</th>
                        <th scope="col">Seed</th>
                        <th scope="col">Playoffs</th>
                        <th scope="col">Division</th>
                        {odds.byes > 0 && <th scope="col">Bye</th>}
                        {odds.rounds.slice(1).map((round) => (
                          <th scope="col" key={round.short}>
                            <abbr title={`Chance to reach the ${round.name}`}>{round.short}</abbr>
                          </th>
                        ))}
                        <th scope="col">Title</th>
                        {odds.mode === 'postseason' && <th scope="col">Status</th>}
                      </tr>
                    </thead>
                    <tbody>
                      {teams.map((team, i) => {
                        // A line under the last listed row would separate nothing.
                        const line = i === odds.playoff_spots - 1 && i < teams.length - 1;
                        return (
                          <tr key={team.id} data-team={team.id} className={line ? 'playoff-line' : undefined}>
                            <th scope="row">
                              {teamName(team.id)}
                              {line && <span className="visually-hidden"> (last playoff spot)</span>}
                              <span className="odds-division">{team.division_label}</span>
                            </th>
                            <td>{formatRecord(team.record)}</td>
                            <td>{team.mean_seed === null ? '—' : team.mean_seed.toFixed(1)}</td>
                            <td>{formatOdds(team.playoffs)}</td>
                            <td>{formatOdds(team.win_division)}</td>
                            {odds.byes > 0 && <td>{formatOdds(team.bye)}</td>}
                            {team.reach.map((p, r) => (
                              <td key={r}>{formatOdds(p)}</td>
                            ))}
                            <td>{formatOdds(team.title)}</td>
                            {odds.mode === 'postseason' && <td>{statusText(team.status, odds.rounds)}</td>}
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                </div>
              </div>
            );
          })}
        </>
      )}
    </section>
  );
}
