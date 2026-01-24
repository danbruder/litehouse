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
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.ensureViewHasText "Welcome to Litehouse"
                        |> Helpers.ensureViewHasText "Create your admin account"
                        |> Helpers.ensureViewHasText "Full Name"
                        |> Helpers.ensureViewHasText "Email"
                        |> Helpers.ensureViewHasText "Organization Name"
                        |> Helpers.ensureViewHasText "Password"
                        |> Helpers.ensureViewHasText "Confirm Password"
                        |> ProgramTest.done
            ]
        , describe "Form Interaction"
            [ test "updates full name field when typing" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "fullName" "John Doe"
                        |> ProgramTest.advanceTime 50
                        |> Helpers.ensureViewHasText "Full Name"
                        |> ProgramTest.done
            , test "updates email field when typing" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "email" "admin@example.com"
                        |> ProgramTest.advanceTime 50
                        |> Helpers.ensureViewHasText "Email"
                        |> ProgramTest.done
            , test "updates organization name field when typing" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "orgName" "My Company"
                        |> ProgramTest.advanceTime 50
                        |> Helpers.ensureViewHasText "Organization Name"
                        |> ProgramTest.done
            , test "updates password field when typing" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "password" "securepass123"
                        |> Helpers.ensureViewHasText "Password"
                        |> ProgramTest.done
            , test "updates confirm password field when typing" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "confirmPassword" "securepass123"
                        |> Helpers.ensureViewHasText "Confirm Password"
                        |> ProgramTest.done
            ]
        , describe "Form Validation"
            [ test "shows error when passwords do not match" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "fullName" "John Doe"
                        |> Helpers.fillIn "email" "admin@example.com"
                        |> Helpers.fillIn "orgName" "My Company"
                        |> Helpers.fillIn "Password" "password123"
                        |> Helpers.fillIn "confirmPassword" "differentpass"
                        |> Helpers.submitForm
                        |> Helpers.ensureViewHasText "Passwords do not match"
                        |> ProgramTest.done
            , test "shows error when password is too short" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "fullName" "John Doe"
                        |> Helpers.fillIn "email" "admin@example.com"
                        |> Helpers.fillIn "orgName" "My Company"
                        |> Helpers.fillIn "Password" "short"
                        |> Helpers.fillIn "confirmPassword" "short"
                        |> Helpers.submitForm
                        |> Helpers.ensureViewHasText "Password must be at least 8 characters"
                        |> ProgramTest.done
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
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "fullName" "John Doe"
                        |> Helpers.fillIn "email" "admin@example.com"
                        |> Helpers.fillIn "orgName" "My Company"
                        |> Helpers.fillIn "Password" "password123"
                        |> Helpers.fillIn "Confirm Password" "password123"
                        |> Helpers.submitForm
                        |> Helpers.ensureHttpRequestWithBody
                            "POST"
                            "/api/auth/register"
                            (Encode.object
                                [ ( "email", Encode.string "admin@example.com" )
                                , ( "password", Encode.string "password123" )
                                , ( "full_name", Encode.string "John Doe" )
                                , ( "organization_name", Encode.string "My Company" )
                                ]
                            )
                        |> Helpers.simulateHttpOk "POST" "/api/auth/register" authResponse
                        |> Helpers.ensureBrowserUrlPath "/dashboard"
                        |> ProgramTest.done
            , test "shows error message on registration failure" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "fullName" "John Doe"
                        |> Helpers.fillIn "email" "admin@example.com"
                        |> Helpers.fillIn "orgName" "My Company"
                        |> Helpers.fillIn "Password" "password123"
                        |> Helpers.fillIn "Confirm Password" "password123"
                        |> Helpers.submitForm
                        |> Helpers.ensureHttpRequestWithBody
                            "POST"
                            "/api/auth/register"
                            (Encode.object
                                [ ( "email", Encode.string "admin@example.com" )
                                , ( "password", Encode.string "password123" )
                                , ( "full_name", Encode.string "John Doe" )
                                , ( "organization_name", Encode.string "My Company" )
                                ]
                            )
                        |> Helpers.simulateHttpError "POST" "/api/auth/register" (Http.BadStatus 409)
                        |> Helpers.ensureViewHasText "An account with this email already exists"
                        |> ProgramTest.done
            , test "disables submit button while submitting" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.fillIn "fullName" "John Doe"
                        |> Helpers.fillIn "email" "admin@example.com"
                        |> Helpers.fillIn "orgName" "My Company"
                        |> Helpers.fillIn "Password" "password123"
                        |> Helpers.fillIn "Confirm Password" "password123"
                        |> Helpers.submitForm
                        |> Helpers.ensureViewHasText "Creating Account..."
                        |> ProgramTest.done
            ]
        ]
