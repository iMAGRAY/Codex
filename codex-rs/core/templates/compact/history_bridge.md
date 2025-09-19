MEMORY_ARCHIVE
GUIDANCE=Эта запись — память прошлых шагов; опирайся на неё для продолжения текущей задачи.
PAST.SUMMARY={{ summary_text }}
PAST.SESSION={% if let Some(value) = session_context_text.as_ref() %}{{ value }}{% else %}(none){% endif %}
PAST.DIRECTIVES={% if let Some(value) = user_instructions_text.as_ref() %}{{ value }}{% else %}(none){% endif %}
PAST.ENV={% if let Some(value) = environment_context_text.as_ref() %}{{ value }}{% else %}(none){% endif %}
PAST.PLAN={% if let Some(value) = plan_text.as_ref() %}{{ value }}{% else %}(none){% endif %}
PAST.REPO={% if let Some(value) = repo_outline_text.as_ref() %}{{ value }}{% else %}(none){% endif %}
PAST.MESSAGES={{ user_messages_text }}
