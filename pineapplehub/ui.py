TABLE_COLUMNS = [
    {
        "name": "parameter",
        "label": "Parameter",
        "field": "parameter",
        "align": "left",
    },
    {"name": "value", "label": "Value", "field": "value"},
]

def reset_table(rows):
    rows[0]["value"] = None
    rows[1]["value"] = None
    rows[2]["value"] = None