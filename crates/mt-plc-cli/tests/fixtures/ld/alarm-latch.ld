{
  "programName": "AlarmLatch",
  "rungs": [
    {
      "id": "r_35",
      "paths": [
        {
          "elements": [
            {
              "id": "e_26",
              "registerType": "X",
              "type": "contact_no",
              "variable": "X0"
            },
            {
              "id": "e_27",
              "type": "coil_set",
              "variable": "M0"
            }
          ],
          "id": "p_28"
        },
        {
          "elements": [
            {
              "id": "e_29",
              "registerType": "X",
              "type": "contact_no",
              "variable": "X1"
            },
            {
              "id": "e_30",
              "type": "coil_set",
              "variable": "M0"
            }
          ],
          "id": "p_31"
        },
        {
          "elements": [
            {
              "id": "e_32",
              "registerType": "X",
              "type": "contact_no",
              "variable": "X2"
            },
            {
              "id": "e_33",
              "type": "coil_set",
              "variable": "M0"
            }
          ],
          "id": "p_34"
        }
      ]
    },
    {
      "id": "r_39",
      "paths": [
        {
          "elements": [
            {
              "id": "e_36",
              "type": "contact_no",
              "variable": "M0"
            },
            {
              "id": "e_37",
              "registerType": "Y",
              "type": "coil",
              "variable": "Y0"
            }
          ],
          "id": "p_38"
        }
      ]
    },
    {
      "id": "r_43",
      "paths": [
        {
          "elements": [
            {
              "id": "e_40",
              "registerType": "X",
              "type": "contact_no",
              "variable": "X3"
            },
            {
              "id": "e_41",
              "type": "coil_reset",
              "variable": "M0"
            }
          ],
          "id": "p_42"
        }
      ]
    }
  ],
  "version": "2.0"
}