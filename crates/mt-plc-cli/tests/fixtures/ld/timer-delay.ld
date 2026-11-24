{
  "programName": "TimerDelay",
  "rungs": [
    {
      "id": "r_13",
      "paths": [
        {
          "elements": [
            {
              "id": "e_10",
              "registerType": "X",
              "type": "contact_no",
              "variable": "X0"
            },
            {
              "blockType": "TON",
              "id": "e_11",
              "instanceName": "delay",
              "pinValues": {
                "PT": "T#500MS"
              },
              "type": "fb_block",
              "variable": ""
            }
          ],
          "id": "p_12"
        }
      ]
    },
    {
      "id": "r_17",
      "paths": [
        {
          "elements": [
            {
              "id": "e_14",
              "registerType": "M",
              "type": "contact_no",
              "variable": "delay.Q"
            },
            {
              "id": "e_15",
              "registerType": "Y",
              "type": "coil",
              "variable": "Y0"
            }
          ],
          "id": "p_16"
        }
      ]
    }
  ],
  "version": "2.0"
}