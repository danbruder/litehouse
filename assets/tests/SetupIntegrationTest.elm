module SetupIntegrationTest exposing (suite)

{-| Integration tests for the Setup page using elm-program-test.
These tests verify the initial setup flow for creating the first admin account.
-}

import Effect
import Expect
import Http
import Json.Encode as Encode
import Main exposing (Flags, Msg(..))
import ProgramTest exposing (ProgramTest)
import ProgramTestHelpers as Helpers
import Test exposing (..)


suite : Test
suite =
    describe "Setup Page Integration Tests"
        [ describe "Initial Load"
            [ test "shows setup form when server is not initialized" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.expectViewHasText "Welcome to Litehouse"
                        |> Helpers.expectViewHasText "Create your admin account"
                        |> Helpers.expectViewHasText "Full Name"
                        |> Helpers.expectViewHasText "Email"
                        |> Helpers.expectViewHasText "Organization Name"
                        |> Helpers.expectViewHasText "Password"
                        |> Helpers.expectViewHasText "Confirm Password"
            ]
        , describe "Form Interaction"
            [ test "updates full name field when typing" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "Full Name" "John Doe"
                        |> Helpers.expectViewHasText "John Doe"
            , test "updates email field when typing" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "Email" "admin@example.com"
                        |> Helpers.expectViewHasText "admin@example.com"
            , test "updates organization name field when typing" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "Organization Name" "My Company"
                        |> Helpers.expectViewHasText "My Company"
            , test "updates password field when typing" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "Password" "securepass123"
                        |> Helpers.expectViewHasText "securepass123"
            , test "updates confirm password field when typing" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "Confirm Password" "securepass123"
                        |> Helpers.expectViewHasText "securepass123"
            ]
        , describe "Form Validation"
            [ test "shows error when passwords do not match" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "Full Name" "John Doe"
                        |> Helpers.fillIn "Email" "admin@example.com"
                        |> Helpers.fillIn "Organization Name" "My Company"
                        |> Helpers.fillIn "Password" "password123"
                        |> Helpers.fillIn "Confirm Password" "differentpass"
                        |> Helpers.submitForm
                        |> Helpers.expectViewHasText "Passwords do not match"
            , test "shows error when password is too short" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "Full Name" "John Doe"
                        |> Helpers.fillIn "Email" "admin@example.com"
                        |> Helpers.fillIn "Organization Name" "My Company"
                        |> Helpers.fillIn "Password" "short"
                        |> Helpers.fillIn "Confirm Password" "short"
                        |> Helpers.submitForm
                        |> Helpers.expectViewHasText "Password must be at least 8 characters"
            ]
        , describe "Form Submission"
            [ test "submits registration form with valid data" <|
                \_ ->
                    let
                        user =
                            { email = "admin@example.com"
                            , fullName = "John Doe"
                            }

                        authResponse =
                            Helpers.authResponseJson "access-token-123" "refresh-token-456" user
                    in
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "Full Name" "John Doe"
                        |> Helpers.fillIn "Email" "admin@example.com"
                        |> Helpers.fillIn "Organization Name" "My Company"
                        |> Helpers.fillIn "Password" "password123"
                        |> Helpers.fillIn "Confirm Password" "password123"
                        |> Helpers.submitForm
                        |> Helpers.expectHttpRequestWithBody
                            "POST"
                            "/api/auth/register"
                            (Encode.object
                                [ ( "email", Encode.string "admin@example.com" )
                                , ( "password", Encode.string "password123" )
                                , ( "full_name", Encode.string "John Doe" )
                                , ( "organization_name", Encode.string "My Company" )
                                ]
                            )
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "POST"
                                , url = "/api/auth/register"
                                , body = ProgramTest.HttpBodyJson
                                    (Encode.object
                                        [ ( "email", Encode.string "admin@example.com" )
                                        , ( "password", Encode.string "password123" )
                                        , ( "full_name", Encode.string "John Doe" )
                                        , ( "organization_name", Encode.string "My Company" )
                                        ]
                                    )
                                , headers = []
                                }
                            )
                            authResponse
                        |> ProgramTest.expectPageChange "/dashboard"
            , test "shows error message on registration failure" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "Full Name" "John Doe"
                        |> Helpers.fillIn "Email" "admin@example.com"
                        |> Helpers.fillIn "Organization Name" "My Company"
                        |> Helpers.fillIn "Password" "password123"
                        |> Helpers.fillIn "Confirm Password" "password123"
                        |> Helpers.submitForm
                        |> Helpers.expectHttpRequestWithBody
                            "POST"
                            "/api/auth/register"
                            (Encode.object
                                [ ( "email", Encode.string "admin@example.com" )
                                , ( "password", Encode.string "password123" )
                                , ( "full_name", Encode.string "John Doe" )
                                , ( "organization_name", Encode.string "My Company" )
                                ]
                            )
                        |> ProgramTest.simulateHttpError
                            (ProgramTest.HttpRequest
                                { method = "POST"
                                , url = "/api/auth/register"
                                , body = ProgramTest.HttpBodyJson
                                    (Encode.object
                                        [ ( "email", Encode.string "admin@example.com" )
                                        , ( "password", Encode.string "password123" )
                                        , ( "full_name", Encode.string "John Doe" )
                                        , ( "organization_name", Encode.string "My Company" )
                                        ]
                                    )
                                , headers = []
                                }
                            )
                            (Http.BadStatus 409)
                        |> Helpers.expectViewHasText "An account with this email already exists"
            , test "disables submit button while submitting" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/auth/status"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "Full Name" "John Doe"
                        |> Helpers.fillIn "Email" "admin@example.com"
                        |> Helpers.fillIn "Organization Name" "My Company"
                        |> Helpers.fillIn "Password" "password123"
                        |> Helpers.fillIn "Confirm Password" "password123"
                        |> Helpers.submitForm
                        |> Helpers.expectViewHasText "Creating Account..."
            ]
        ]
